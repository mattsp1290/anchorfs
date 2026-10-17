use crate::{
    config::{Addressing, ConfigError, CredentialSource, S3Config},
    key::ObjectKey,
    traits::{BackendError, ListPage, ObjectBackend, ObjectMeta, PutOutcome},
};
use anchorfs_core::digest::Sha256Digest;
use async_trait::async_trait;
use aws_credential_types::provider::{ProvideCredentials, future};
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, SharedCredentialsProvider, retry::RetryConfig,
    timeout::TimeoutConfig,
};
use aws_smithy_http_client::{
    Builder as HttpClientBuilder,
    tls::{Provider, TlsContext, TrustStore, rustls_provider::CryptoMode},
};
use bytes::Bytes;
use std::ops::Range;

#[derive(Debug, thiserror::Error)]
pub enum S3BuildError {
    #[error(transparent)]
    InvalidConfig(#[from] ConfigError),
    #[error("cannot read CA bundle")]
    CaBundle(#[source] std::io::Error),
    #[error("invalid TLS configuration")]
    TlsConfiguration,
}
/// SDK configuration only. All object operations remain Stage 0 stubs.
#[derive(Debug, Clone)]
pub struct S3Backend {
    client: aws_sdk_s3::Client,
}
/// Delay chain construction and credential resolution until an actual request.
#[derive(Debug)]
struct LazyChain {
    region: Region,
}
impl ProvideCredentials for LazyChain {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async move {
            aws_config::default_provider::credentials::DefaultCredentialsChain::builder()
                .region(self.region.clone())
                .build()
                .await
                .provide_credentials()
                .await
        })
    }
}
impl S3Backend {
    /// Performs local configuration and optional CA file reads, never network I/O.
    pub fn from_config(config: &S3Config) -> Result<Self, S3BuildError> {
        config.validate()?;
        let region = Region::new(config.region.clone());
        let credentials = match &config.credential_source {
            CredentialSource::Environment => SharedCredentialsProvider::new(
                aws_config::environment::credentials::EnvironmentVariableCredentialsProvider::new(),
            ),
            CredentialSource::Static {
                access_key_id,
                secret_access_key,
            } => SharedCredentialsProvider::new(Credentials::new(
                access_key_id.clone(),
                secret_access_key.expose().clone(),
                None,
                None,
                "anchorfs-config",
            )),
            CredentialSource::Chain => SharedCredentialsProvider::new(LazyChain {
                region: region.clone(),
            }),
        };
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(config.endpoint.as_str())
            .region(region)
            .force_path_style(config.addressing == Addressing::Path)
            .credentials_provider(credentials)
            // SDK max_attempts counts the initial attempt as well as retries.
            .retry_config(RetryConfig::standard().with_max_attempts(config.max_retries + 1))
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(config.request_timeout)
                    .operation_attempt_timeout(config.request_timeout)
                    .build(),
            );
        if let Some(path) = &config.ca_bundle_path {
            let pem = std::fs::read(path).map_err(S3BuildError::CaBundle)?;
            let tls = TlsContext::builder()
                .with_trust_store(TrustStore::default().with_pem_certificate(pem))
                .build()
                .map_err(|_| S3BuildError::TlsConfiguration)?;
            builder = builder.http_client(
                HttpClientBuilder::new()
                    .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
                    .tls_context(tls)
                    .build_https(),
            );
        }
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(builder.build()),
        })
    }
    pub fn client(&self) -> &aws_sdk_s3::Client {
        &self.client
    }
}
#[async_trait]
impl ObjectBackend for S3Backend {
    async fn put(
        &self,
        _key: &ObjectKey,
        _bytes: Bytes,
        _expected_sha256: Sha256Digest,
    ) -> Result<PutOutcome, BackendError> {
        Err(BackendError::NotImplemented)
    }
    async fn get_range(&self, _key: &ObjectKey, _range: Range<u64>) -> Result<Bytes, BackendError> {
        Err(BackendError::NotImplemented)
    }
    async fn head(&self, _key: &ObjectKey) -> Result<ObjectMeta, BackendError> {
        Err(BackendError::NotImplemented)
    }
    async fn list(
        &self,
        _prefix: &str,
        _cursor: Option<&str>,
        _limit: u32,
    ) -> Result<ListPage, BackendError> {
        Err(BackendError::NotImplemented)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use anchorfs_core::ids::{ObjectId, WorkspaceId};
    #[test]
    fn explicit_configuration_without_runtime_or_network() {
        let mut config = crate::config::local_config();
        for credentials in [
            config.credential_source.clone(),
            CredentialSource::Environment,
            CredentialSource::Chain,
        ] {
            config.credential_source = credentials;
            for addressing in [Addressing::Path, Addressing::VirtualHost] {
                config.addressing = addressing;
                let backend = S3Backend::from_config(&config).unwrap();
                let sdk = backend.client().config();
                assert_eq!(sdk.region().unwrap().as_ref(), "local");
                assert_eq!(sdk.retry_config().unwrap().max_attempts(), 1);
                assert_eq!(
                    sdk.timeout_config().unwrap().operation_timeout(),
                    Some(config.request_timeout)
                );
            }
        }
    }
    #[test]
    fn invalid_configuration_is_rejected_before_client_construction() {
        let mut config = crate::config::local_config();
        config.bucket.clear();
        assert!(matches!(
            S3Backend::from_config(&config),
            Err(S3BuildError::InvalidConfig(_))
        ));
        config = crate::config::local_config();
        config.ca_bundle_path = Some("/nonexistent-anchorfs-test-ca.pem".into());
        assert!(matches!(
            S3Backend::from_config(&config),
            Err(S3BuildError::CaBundle(_))
        ));
    }
    #[tokio::test]
    async fn all_operations_are_network_free_stubs() {
        let config = crate::config::local_config();
        let backend: Box<dyn ObjectBackend> = Box::new(S3Backend::from_config(&config).unwrap());
        let key = ObjectKey::new(&config.deployment_id, WorkspaceId::new(), ObjectId::new());
        assert_eq!(
            backend
                .put(
                    &key,
                    Bytes::from_static(b"abc"),
                    anchorfs_core::digest::sha256(b"abc")
                )
                .await,
            Err(BackendError::NotImplemented)
        );
        assert_eq!(
            backend.get_range(&key, 0..1).await,
            Err(BackendError::NotImplemented)
        );
        assert_eq!(backend.head(&key).await, Err(BackendError::NotImplemented));
        assert_eq!(
            backend.list("local-test/", None, 10).await,
            Err(BackendError::NotImplemented)
        );
    }
}
