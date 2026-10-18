use anchorfs_core::{ids::DeploymentId, secret::Secret};
use std::{path::PathBuf, time::Duration};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addressing {
    Path,
    VirtualHost,
}
#[derive(Debug, Clone)]
pub enum CredentialSource {
    Environment,
    Static {
        access_key_id: String,
        secret_access_key: Secret<String>,
    },
    Chain,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumMode {
    Sha256,
    None,
}
#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: Url,
    pub checksum_mode: ChecksumMode,
    pub region: String,
    pub addressing: Addressing,
    pub bucket: String,
    pub deployment_id: DeploymentId,
    pub ca_bundle_path: Option<PathBuf>,
    pub credential_source: CredentialSource,
    pub request_timeout: Duration,
    pub max_retries: u32,
    /// Explicit development profile; production endpoints require HTTPS.
    pub allow_plaintext: bool,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid S3 configuration: {0}")]
pub struct ConfigError(pub &'static str);
impl S3Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.bucket.trim().is_empty() {
            return Err(ConfigError("bucket must not be empty"));
        }
        if self.region.trim().is_empty() {
            return Err(ConfigError("region must not be empty"));
        }
        if self.endpoint.host_str().is_none()
            || !self.endpoint.username().is_empty()
            || self.endpoint.password().is_some()
            || self.endpoint.query().is_some()
            || self.endpoint.fragment().is_some()
        {
            return Err(ConfigError(
                "endpoint must have a host and no embedded credentials, query or fragment",
            ));
        }
        match self.endpoint.scheme() {
            "https" => {}
            "http" if self.allow_plaintext => {}
            _ => {
                return Err(ConfigError(
                    "HTTPS is required unless the explicit plaintext development profile is enabled",
                ));
            }
        }
        if self.max_retries > 10 {
            return Err(ConfigError("max_retries must be at most 10"));
        }
        if !(Duration::from_secs(1)..=Duration::from_secs(300)).contains(&self.request_timeout) {
            return Err(ConfigError(
                "request_timeout must be between 1 and 300 seconds",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
pub(crate) fn local_config() -> S3Config {
    S3Config {
        endpoint: "http://127.0.0.1:1".parse().unwrap(),
        checksum_mode: ChecksumMode::Sha256,
        region: "local".into(),
        addressing: Addressing::Path,
        bucket: "synthetic-test".into(),
        deployment_id: DeploymentId::parse("local-test").unwrap(),
        ca_bundle_path: None,
        credential_source: CredentialSource::Static {
            access_key_id: "synthetic-id".into(),
            secret_access_key: Secret::new("synthetic-secret".into()),
        },
        request_timeout: Duration::from_secs(1),
        max_retries: 0,
        allow_plaintext: true,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_boundaries() {
        let mut c = local_config();
        assert!(c.validate().is_ok());
        c.allow_plaintext = false;
        assert!(c.validate().is_err());
        c.endpoint = "https://objects.invalid".parse().unwrap();
        assert!(c.validate().is_ok());
        for bad in [
            "ftp://objects.invalid",
            "https://user:secret@objects.invalid",
            "https://objects.invalid/?key=x",
        ] {
            c.endpoint = bad.parse().unwrap();
            assert!(c.validate().is_err());
        }
        c = local_config();
        c.bucket.clear();
        assert!(c.validate().is_err());
        c = local_config();
        for duration in [
            Duration::ZERO,
            Duration::from_millis(999),
            Duration::from_secs(301),
        ] {
            c.request_timeout = duration;
            assert!(c.validate().is_err());
        }
        for duration in [Duration::from_secs(1), Duration::from_secs(300)] {
            c.request_timeout = duration;
            assert!(c.validate().is_ok());
        }
        c.max_retries = 10;
        assert!(c.validate().is_ok());
        c.max_retries = 11;
        assert!(c.validate().is_err());
    }
}
