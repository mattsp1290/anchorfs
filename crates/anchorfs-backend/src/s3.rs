use crate::{
    config::{Addressing, ChecksumMode, ConfigError, CredentialSource, S3Config},
    key::ObjectKey,
    traits::{BackendError, ListPage, ObjectBackend, ObjectMeta, PutOutcome},
};
use anchorfs_core::{
    Limits,
    digest::{Sha256Digest, sha256},
    ids::DeploymentId,
};
use async_trait::async_trait;
use aws_credential_types::provider::{ProvideCredentials, future};
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
    SharedCredentialsProvider,
    retry::{RetryConfig, RetryPartition},
    timeout::TimeoutConfig,
};
use aws_sdk_s3::{
    error::{ProvideErrorMetadata, SdkError},
    primitives::ByteStream,
    types::{ChecksumAlgorithm, ChecksumMode as SdkChecksumMode},
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
/// Bounded immutable object operations over the configured S3 provider.
#[derive(Debug, Clone)]
pub struct S3Backend {
    client: aws_sdk_s3::Client,
    probe_client: aws_sdk_s3::Client,
    bucket: String,
    deployment: DeploymentId,
    prefix: String,
    checksum_mode: ChecksumMode,
    max_object_bytes: u64,
    request_timeout: std::time::Duration,
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
            .retry_partition(RetryPartition::new(format!(
                "anchorfs-{}",
                config.deployment_id
            )))
            .request_checksum_calculation(match config.checksum_mode {
                ChecksumMode::Sha256 => RequestChecksumCalculation::WhenSupported,
                ChecksumMode::None => RequestChecksumCalculation::WhenRequired,
            })
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
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
        let probe_builder = builder
            .clone()
            .retry_config(RetryConfig::disabled())
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(
                        config
                            .request_timeout
                            .min(std::time::Duration::from_secs(1)),
                    )
                    .operation_attempt_timeout(
                        config
                            .request_timeout
                            .min(std::time::Duration::from_secs(1)),
                    )
                    .build(),
            );
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(builder.build()),
            probe_client: aws_sdk_s3::Client::from_conf(probe_builder.build()),
            bucket: config.bucket.clone(),
            deployment: config.deployment_id.clone(),
            prefix: ObjectKey::deployment_prefix(&config.deployment_id),
            checksum_mode: config.checksum_mode,
            max_object_bytes: u64::from(Limits::default().chunk_size),
            request_timeout: config.request_timeout,
        })
    }
    pub fn client(&self) -> &aws_sdk_s3::Client {
        &self.client
    }
}
impl S3Backend {
    fn guard(&self, key: &ObjectKey) -> Result<(), BackendError> {
        if key.as_str().starts_with(&self.prefix) {
            Ok(())
        } else {
            Err(BackendError::Other)
        }
    }
    async fn head_with_checksum(
        &self,
        key: &ObjectKey,
    ) -> Result<(ObjectMeta, Option<String>), BackendError> {
        self.guard(key)?;
        let r = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .checksum_mode(SdkChecksumMode::Enabled)
            .send()
            .await
            .map_err(|e| read_error(&e, true))?;
        let length = length(r.content_length())?;
        Ok((
            ObjectMeta {
                key: key.clone(),
                length,
            },
            r.checksum_sha256().map(str::to_owned),
        ))
    }
    async fn upload(
        &self,
        key: &ObjectKey,
        bytes: Bytes,
        checksum: &str,
    ) -> Result<Option<String>, Classified> {
        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .body(ByteStream::from(bytes));
        if self.checksum_mode == ChecksumMode::Sha256 {
            req = req
                .checksum_algorithm(ChecksumAlgorithm::Sha256)
                .checksum_sha256(checksum);
        }
        req.send()
            .await
            .map(|r| r.checksum_sha256().map(str::to_owned))
            .map_err(|e| classify_put(&e))
    }
    async fn confirm(
        &self,
        key: &ObjectKey,
        digest: Sha256Digest,
        size: u64,
        echo: Option<String>,
    ) -> Result<(), BackendError> {
        if self.checksum_mode == ChecksumMode::Sha256 {
            match echo {
                Some(e) if e == base64_digest(digest) => (),
                Some(_) => return Err(BackendError::IntegrityMismatch),
                None => {
                    self.get_verified(key, digest, size).await?;
                }
            }
        }
        Ok(())
    }
}
#[async_trait]
impl ObjectBackend for S3Backend {
    async fn put(
        &self,
        key: &ObjectKey,
        bytes: Bytes,
        expected_sha256: Sha256Digest,
    ) -> Result<PutOutcome, BackendError> {
        self.guard(key)?;
        let size = bytes.len() as u64;
        if size == 0 {
            return Err(BackendError::Other);
        }
        if size > self.max_object_bytes {
            return Err(BackendError::ObjectTooLarge);
        }
        if sha256(&bytes) != expected_sha256 {
            return Err(BackendError::IntegrityMismatch);
        }
        let outcome = PutOutcome {
            bytes_written: size,
            sha256: expected_sha256,
        };
        let checksum = base64_digest(expected_sha256);
        for round in 0..2 {
            match self.upload(key, bytes.clone(), &checksum).await {
                Ok(echo) => {
                    self.confirm(key, expected_sha256, size, echo).await?;
                    return Ok(outcome);
                }
                Err(Classified::Definitive(e)) => return Err(e),
                Err(Classified::Ambiguous) => (),
            }
            match self.head_with_checksum(key).await {
                Ok((meta, echo)) => {
                    if meta.length != size {
                        return Err(BackendError::IntegrityMismatch);
                    }
                    if self.checksum_mode == ChecksumMode::Sha256 && echo.is_some() {
                        self.confirm(key, expected_sha256, size, echo).await?;
                    } else {
                        self.get_verified(key, expected_sha256, size).await?;
                    }
                    return Ok(outcome);
                }
                Err(BackendError::NotFound) if round == 0 => (),
                Err(BackendError::NotFound) => return Err(BackendError::Unavailable),
                Err(_) => return Err(BackendError::UncertainOutcome),
            }
        }
        unreachable!("two bounded upload rounds return")
    }
    async fn get_range(&self, key: &ObjectKey, range: Range<u64>) -> Result<Bytes, BackendError> {
        self.guard(key)?;
        if range.start >= range.end || range.end - range.start > self.max_object_bytes {
            return Err(BackendError::InvalidRange);
        }
        let size = range.end - range.start;
        let deadline = tokio::time::Instant::now() + self.request_timeout;
        let r = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .range(format!("bytes={}-{}", range.start, range.end - 1))
            .send()
            .await
            .map_err(|e| read_error(&e, true))?;
        let expected = format!("bytes {}-{}/", range.start, range.end - 1);
        if !r.content_range().is_some_and(|v| {
            v.starts_with(&expected)
                && v[expected.len()..]
                    .parse::<u64>()
                    .is_ok_and(|total| total >= range.end)
        }) {
            return Err(BackendError::InvalidRange);
        }
        if r.content_length()
            .is_some_and(|n| u64::try_from(n) != Ok(size))
        {
            return Err(BackendError::IntegrityMismatch);
        }
        let body = tokio::time::timeout_at(deadline, collect_bounded(r.body, size))
            .await
            .map_err(|_| BackendError::Timeout)??;
        if body.len() as u64 != size {
            return Err(BackendError::IntegrityMismatch);
        }
        Ok(body)
    }
    async fn get_verified(
        &self,
        key: &ObjectKey,
        expected_sha256: Sha256Digest,
        expected_length: u64,
    ) -> Result<Bytes, BackendError> {
        self.guard(key)?;
        if expected_length == 0 {
            return Err(BackendError::Other);
        }
        if expected_length > self.max_object_bytes {
            return Err(BackendError::ObjectTooLarge);
        }
        let deadline = tokio::time::Instant::now() + self.request_timeout;
        let r = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send()
            .await
            .map_err(|e| read_error(&e, true))?;
        if r.content_length()
            .is_some_and(|n| u64::try_from(n) != Ok(expected_length))
        {
            return Err(BackendError::IntegrityMismatch);
        }
        let body = tokio::time::timeout_at(deadline, collect_bounded(r.body, expected_length))
            .await
            .map_err(|_| BackendError::Timeout)??;
        if body.len() as u64 != expected_length || sha256(&body) != expected_sha256 {
            return Err(BackendError::IntegrityMismatch);
        }
        Ok(body)
    }
    async fn head(&self, key: &ObjectKey) -> Result<ObjectMeta, BackendError> {
        self.head_with_checksum(key).await.map(|(m, _)| m)
    }
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ListPage, BackendError> {
        if !prefix.starts_with(&self.prefix) {
            return Err(BackendError::Other);
        }
        let r = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .max_keys(limit.clamp(1, 1000) as i32)
            .set_continuation_token(cursor.map(str::to_owned))
            .send()
            .await
            .map_err(|e| read_error(&e, false))?;
        let objects = r
            .contents()
            .iter()
            .map(|o| {
                Ok(ObjectMeta {
                    key: ObjectKey::parse(o.key().ok_or(BackendError::Other)?, &self.deployment)
                        .map_err(|_| BackendError::Other)?,
                    length: length(o.size())?,
                })
            })
            .collect::<Result<_, BackendError>>()?;
        Ok(ListPage {
            objects,
            next_cursor: if r.is_truncated() == Some(true) {
                Some(
                    r.next_continuation_token()
                        .ok_or(BackendError::Other)?
                        .into(),
                )
            } else {
                None
            },
        })
    }
    async fn probe(&self) -> Result<(), BackendError> {
        self.probe_client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .max_keys(1)
            .send()
            .await
            .map_err(|e| read_error(&e, false))?;
        Ok(())
    }
}
// Enforce the caller's bound even if an intermediary omits Content-Length.
async fn collect_bounded(mut body: ByteStream, expected: u64) -> Result<Bytes, BackendError> {
    let mut bytes = bytes::BytesMut::new();
    while let Some(chunk) = body
        .try_next()
        .await
        .map_err(|_| BackendError::Unavailable)?
    {
        if chunk.len() as u64 > expected.saturating_sub(bytes.len() as u64) {
            return Err(BackendError::IntegrityMismatch);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes.freeze())
}
fn length(n: Option<i64>) -> Result<u64, BackendError> {
    n.and_then(|v| u64::try_from(v).ok())
        .ok_or(BackendError::Other)
}
#[derive(Debug, PartialEq, Eq)]
enum Classified {
    Definitive(BackendError),
    Ambiguous,
}
fn service_error(
    code: Option<&str>,
    status: Option<u16>,
    message: Option<&str>,
    object: bool,
) -> BackendError {
    match code.unwrap_or("") {
        "NoSuchBucket" => BackendError::BucketNotFound,
        "NoSuchKey" | "NotFound" if object => BackendError::NotFound,
        "InvalidRange" => BackendError::InvalidRange,
        "AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch" => BackendError::Forbidden,
        "BadDigest"
        | "XAmzContentSHA256Mismatch"
        | "XAmzContentChecksumMismatch"
        | "InvalidDigest" => BackendError::IntegrityMismatch,
        "InvalidRequest"
            if message.is_some_and(|s| s.to_ascii_lowercase().contains("checksum")) =>
        {
            BackendError::IntegrityMismatch
        }
        "RequestTimeout" | "IncompleteBody" => BackendError::Unavailable,
        _ => match status {
            Some(401 | 403) => BackendError::Forbidden,
            Some(404) if object && code.is_none() => BackendError::NotFound,
            Some(404) => BackendError::BucketNotFound,
            Some(416) => BackendError::InvalidRange,
            Some(429 | 500..=599) => BackendError::Unavailable,
            _ => BackendError::Other,
        },
    }
}
fn read_error<E: ProvideErrorMetadata>(e: &SdkError<E>, object: bool) -> BackendError {
    match e {
        SdkError::ConstructionFailure(_) => BackendError::Other,
        SdkError::TimeoutError(_) => BackendError::Timeout,
        SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => BackendError::Unavailable,
        _ => service_error(
            e.code(),
            e.raw_response().map(|r| r.status().as_u16()),
            e.message(),
            object,
        ),
    }
}
fn classify_put<E: ProvideErrorMetadata>(e: &SdkError<E>) -> Classified {
    match e {
        SdkError::ConstructionFailure(_) => Classified::Definitive(BackendError::Other),
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            Classified::Ambiguous
        }
        _ if e
            .raw_response()
            .is_some_and(|r| r.status().as_u16() == 429 || r.status().as_u16() >= 500) =>
        {
            Classified::Ambiguous
        }
        _ => Classified::Definitive(read_error(e, false)),
    }
}
fn base64_digest(d: Sha256Digest) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(44);
    for chunk in d.0.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(ALPHABET[a >> 2] as char);
        out.push(ALPHABET[((a & 3) << 4) | (b >> 4)] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[c & 63] as char
        } else {
            '='
        });
    }
    out
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

    fn key(config: &S3Config) -> ObjectKey {
        ObjectKey::new(&config.deployment_id, WorkspaceId::new(), ObjectId::new())
    }
    #[tokio::test]
    async fn local_validation_never_sends_requests() {
        let c = crate::config::local_config();
        let b = S3Backend::from_config(&c).unwrap();
        let k = key(&c);
        let start = std::time::Instant::now();
        assert_eq!(
            b.put(&k, Bytes::from_static(b"abc"), sha256(b"other"))
                .await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(
            b.put(&k, Bytes::new(), sha256(b"")).await,
            Err(BackendError::Other)
        );
        assert_eq!(
            b.put(
                &k,
                Bytes::from(vec![0; b.max_object_bytes as usize + 1]),
                sha256(b"")
            )
            .await,
            Err(BackendError::ObjectTooLarge)
        );
        for r in [0..0, Range { start: 4, end: 3 }, 0..b.max_object_bytes + 1] {
            assert_eq!(b.get_range(&k, r).await, Err(BackendError::InvalidRange));
        }
        assert_eq!(
            b.get_verified(&k, sha256(b""), 0).await,
            Err(BackendError::Other)
        );
        assert_eq!(
            b.get_verified(&k, sha256(b""), b.max_object_bytes + 1)
                .await,
            Err(BackendError::ObjectTooLarge)
        );
        assert_eq!(b.list("foreign/", None, 10).await, Err(BackendError::Other));
        let k = ObjectKey::new(
            &DeploymentId::parse("foreign").unwrap(),
            WorkspaceId::new(),
            ObjectId::new(),
        );
        assert_eq!(b.head(&k).await, Err(BackendError::Other));
        assert_eq!(b.get_range(&k, 0..1).await, Err(BackendError::Other));
        assert_eq!(
            b.get_verified(&k, sha256(b"a"), 1).await,
            Err(BackendError::Other)
        );
        assert_eq!(
            b.put(&k, Bytes::from_static(b"a"), sha256(b"a")).await,
            Err(BackendError::Other)
        );
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }
    #[test]
    fn base64_digest_known_vector() {
        assert_eq!(
            base64_digest(sha256(b"abc")),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
    }
    #[tokio::test]
    async fn unroutable_endpoint_is_unavailable_or_timeout() {
        let c = crate::config::local_config();
        let b = S3Backend::from_config(&c).unwrap();
        let k = key(&c);
        let start = std::time::Instant::now();
        assert!(matches!(
            b.head(&k).await,
            Err(BackendError::Unavailable | BackendError::Timeout)
        ));
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert_eq!(
            b.put(&k, Bytes::from_static(b"a"), sha256(b"a")).await,
            Err(BackendError::UncertainOutcome)
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(6));
    }
    #[test]
    fn classify_table() {
        use aws_sdk_s3::{
            config::http::HttpResponse, error::ErrorMetadata, operation::put_object::PutObjectError,
        };
        let cases = [
            ("NoSuchBucket", 404, BackendError::BucketNotFound),
            ("AccessDenied", 403, BackendError::Forbidden),
            ("InvalidAccessKeyId", 400, BackendError::Forbidden),
            ("SignatureDoesNotMatch", 403, BackendError::Forbidden),
            ("BadDigest", 400, BackendError::IntegrityMismatch),
            (
                "XAmzContentSHA256Mismatch",
                400,
                BackendError::IntegrityMismatch,
            ),
            (
                "XAmzContentChecksumMismatch",
                400,
                BackendError::IntegrityMismatch,
            ),
            ("InvalidDigest", 400, BackendError::IntegrityMismatch),
            ("InvalidRequest", 400, BackendError::IntegrityMismatch),
            ("RequestTimeout", 400, BackendError::Unavailable),
            ("IncompleteBody", 400, BackendError::Unavailable),
            ("SlowDown", 503, BackendError::Unavailable),
            ("InternalError", 500, BackendError::Unavailable),
            ("Unknown", 429, BackendError::Unavailable),
            ("Unknown", 401, BackendError::Forbidden),
            ("Unknown", 400, BackendError::Other),
            ("InvalidRange", 416, BackendError::InvalidRange),
        ];
        for (code, status, expected) in cases {
            let e = SdkError::service_error(
                PutObjectError::generic(
                    ErrorMetadata::builder()
                        .code(code)
                        .message("checksum synthetic")
                        .build(),
                ),
                HttpResponse::new(
                    status.try_into().unwrap(),
                    ByteStream::from_static(b"").into_inner(),
                ),
            );
            assert_eq!(read_error(&e, false), expected, "{code}");
            assert_eq!(
                classify_put(&e),
                if status == 429 || status >= 500 {
                    Classified::Ambiguous
                } else {
                    Classified::Definitive(expected)
                },
                "{code}"
            );
        }
        assert_eq!(
            service_error(Some("NoSuchKey"), Some(404), None, true),
            BackendError::NotFound
        );
        assert_eq!(
            service_error(Some("NotFound"), Some(404), None, true),
            BackendError::NotFound
        );
        assert_eq!(
            service_error(Some("UnexpectedBucketError"), Some(404), None, true),
            BackendError::BucketNotFound
        );
        let e: SdkError<PutObjectError> =
            SdkError::construction_failure(std::io::Error::other("synthetic"));
        assert_eq!(read_error(&e, false), BackendError::Other);
        assert_eq!(
            classify_put(&e),
            Classified::Definitive(BackendError::Other)
        );
        let e: SdkError<PutObjectError> =
            SdkError::timeout_error(std::io::Error::other("synthetic"));
        assert_eq!(read_error(&e, false), BackendError::Timeout);
        assert_eq!(classify_put(&e), Classified::Ambiguous);
        let e: SdkError<PutObjectError> = SdkError::dispatch_failure(
            aws_sdk_s3::error::ConnectorError::io(Box::new(std::io::Error::other("synthetic"))),
        );
        assert_eq!(read_error(&e, false), BackendError::Unavailable);
        assert_eq!(classify_put(&e), Classified::Ambiguous);
        let e: SdkError<PutObjectError> = SdkError::response_error(
            std::io::Error::other("synthetic"),
            HttpResponse::new(
                502u16.try_into().unwrap(),
                ByteStream::from_static(b"").into_inner(),
            ),
        );
        assert_eq!(read_error(&e, false), BackendError::Unavailable);
        assert_eq!(classify_put(&e), Classified::Ambiguous);
    }
}
