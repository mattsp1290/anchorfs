use anchorfs_backend::config::{Addressing, ChecksumMode, CredentialSource, S3Config};
use anchorfs_core::{DeploymentId, Limits, Secret, Timers, random_token};
use anchorfs_persistence::{
    meta::DeploymentQuotaConfig, pool::PoolConfig, settings::DurabilityProfile,
};
use std::{env, net::SocketAddr, sync::Arc, time::Duration};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: Secret<String>,
    pub deployment_id: DeploymentId,
    pub listen_addr: SocketAddr,
    pub durability_profile: DurabilityProfile,
    pub db_max_connections: u32,
    pub client_session_ttl: Duration,
    pub receipt_retention: Duration,
    pub db_lock_timeout: Duration,
    pub max_sessions_per_credential: u32,
    pub metadata_call_deadline: Duration,
    pub data_call_deadline: Duration,
    pub shutdown_drain: Duration,
    pub max_request_bytes: usize,
    pub max_replace_bytes: usize,
    pub handle_ttl: Duration,
    pub max_handles_per_session: u32,
    pub cursor_ttl: Duration,
    pub max_list_page: u32,
    pub cursor_signing_key: Secret<String>,
    pub deployment_physical_bytes_limit: i64,
    pub deployment_staging_bytes_limit: i64,
    pub s3: S3Config,
    pub backend_probe_deadline: Duration,
}
#[derive(Debug)]
pub struct ConfigError {
    pub code: i32,
    pub message: String,
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ConfigError {}
fn invalid(name: &str) -> ConfigError {
    ConfigError {
        code: 2,
        message: format!("invalid or missing ANCHORFS_{name}"),
    }
}
fn value(name: &str) -> Option<String> {
    env::var(format!("ANCHORFS_{name}")).ok()
}
fn parse<T: std::str::FromStr>(name: &str, default: T) -> Result<T, ConfigError> {
    match value(name) {
        Some(v) => v.parse().map_err(|_| invalid(name)),
        None => Ok(default),
    }
}
impl Config {
    /// Pure defaults used to construct the contract. No credentials are resolved and no connection is opened.
    pub fn contract_defaults() -> Self {
        let limits = Limits::default();
        let timers = Timers::default();
        let deployment_id = DeploymentId::parse("local").expect("static id");
        Self {
            database_url: Secret::new("postgres://unused:unused@127.0.0.1:1/unused".into()),
            deployment_id: deployment_id.clone(),
            listen_addr: "127.0.0.1:8080".parse().expect("static address"),
            durability_profile: DurabilityProfile::Durable,
            db_max_connections: 8,
            client_session_ttl: timers.client_session_ttl,
            receipt_retention: timers.receipt_retention,
            db_lock_timeout: Duration::from_secs(30),
            max_sessions_per_credential: limits.max_sessions_per_credential,
            metadata_call_deadline: timers.metadata_call_deadline,
            data_call_deadline: timers.data_call_deadline,
            shutdown_drain: timers.shutdown_drain,
            max_request_bytes: limits.max_api_write as usize,
            max_replace_bytes: limits.max_replace_bytes as usize,
            handle_ttl: timers.handle_ttl,
            max_handles_per_session: limits.max_handles_per_session,
            cursor_ttl: timers.cursor_ttl,
            max_list_page: limits.max_list_page,
            // Fixed test-only key; from_env always replaces it.
            cursor_signing_key: Secret::new("0123456789abcdef".repeat(4)),
            deployment_physical_bytes_limit: 1 << 40,
            deployment_staging_bytes_limit: 1 << 30,
            backend_probe_deadline: Duration::from_secs(1),
            s3: S3Config {
                checksum_mode: anchorfs_backend::config::ChecksumMode::Sha256,
                endpoint: "https://s3.invalid".parse().expect("static URL"),
                region: "local".into(),
                addressing: Addressing::Path,
                bucket: "anchorfs".into(),
                deployment_id,
                ca_bundle_path: None,
                credential_source: CredentialSource::Environment,
                request_timeout: Duration::from_secs(60),
                max_retries: 3,
                allow_plaintext: false,
            },
        }
    }
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut c = Self::contract_defaults();
        c.durability_profile = match value("DURABILITY_PROFILE").as_deref().unwrap_or("durable") {
            "durable" => DurabilityProfile::Durable,
            "development" => DurabilityProfile::Development,
            _ => return Err(invalid("DURABILITY_PROFILE")),
        };
        c.listen_addr = parse("LISTEN_ADDR", c.listen_addr)?;
        if !c.listen_addr.ip().is_loopback() && c.durability_profile == DurabilityProfile::Durable {
            return Err(ConfigError {
                code: 7,
                message: "durable plaintext listener must use loopback".into(),
            });
        }
        if value("TLS_CERT_PATH").is_some() || value("TLS_KEY_PATH").is_some() {
            return Err(ConfigError {
                code: 2,
                message: "TLS is not implemented in this version".into(),
            });
        }
        c.database_url = Secret::new(value("DATABASE_URL").ok_or_else(|| invalid("DATABASE_URL"))?);
        c.deployment_id =
            DeploymentId::parse(value("DEPLOYMENT_ID").ok_or_else(|| invalid("DEPLOYMENT_ID"))?)
                .map_err(|_| invalid("DEPLOYMENT_ID"))?;
        c.cursor_signing_key = match value("CURSOR_SIGNING_KEY") {
            Some(key) if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Secret::new(key)
            }
            Some(_) => return Err(invalid("CURSOR_SIGNING_KEY")),
            None if c.durability_profile == DurabilityProfile::Development => {
                tracing::warn!("using an ephemeral cursor signing key");
                random_token()
            }
            None => return Err(invalid("CURSOR_SIGNING_KEY")),
        };
        c.handle_ttl =
            Duration::from_millis(parse("HANDLE_TTL_MS", c.handle_ttl.as_millis() as u64)?);
        c.cursor_ttl =
            Duration::from_millis(parse("CURSOR_TTL_MS", c.cursor_ttl.as_millis() as u64)?);
        c.max_handles_per_session = parse("MAX_HANDLES_PER_SESSION", c.max_handles_per_session)?;
        c.max_list_page = parse("MAX_LIST_PAGE", c.max_list_page)?;
        c.db_max_connections = parse("DB_MAX_CONNECTIONS", c.db_max_connections)?;
        c.metadata_call_deadline = Duration::from_millis(parse(
            "METADATA_CALL_DEADLINE_MS",
            c.metadata_call_deadline.as_millis() as u64,
        )?);
        c.client_session_ttl = Duration::from_millis(parse(
            "CLIENT_SESSION_TTL_MS",
            c.client_session_ttl.as_millis() as u64,
        )?);
        c.receipt_retention = Duration::from_millis(parse(
            "RECEIPT_RETENTION_MS",
            c.receipt_retention.as_millis() as u64,
        )?);
        c.db_lock_timeout = Duration::from_millis(parse(
            "DB_LOCK_TIMEOUT_MS",
            c.db_lock_timeout.as_millis() as u64,
        )?);
        c.data_call_deadline = Duration::from_millis(parse(
            "DATA_CALL_DEADLINE_MS",
            c.data_call_deadline.as_millis() as u64,
        )?);
        c.max_request_bytes = parse("MAX_REQUEST_BYTES", c.max_request_bytes)?;
        c.max_replace_bytes = parse("MAX_REPLACE_BYTES", c.max_replace_bytes)?;
        c.deployment_physical_bytes_limit = parse(
            "DEPLOYMENT_PHYSICAL_BYTES_LIMIT",
            c.deployment_physical_bytes_limit,
        )?;
        c.deployment_staging_bytes_limit = parse(
            "DEPLOYMENT_STAGING_BYTES_LIMIT",
            c.deployment_staging_bytes_limit,
        )?;
        if c.handle_ttl.is_zero()
            || c.cursor_ttl.is_zero()
            || c.max_handles_per_session == 0
            || c.max_list_page == 0
            || c.client_session_ttl.is_zero()
            || c.receipt_retention.is_zero()
            || c.db_lock_timeout.is_zero()
            || c.db_max_connections == 0
            || c.metadata_call_deadline.is_zero()
            || c.data_call_deadline.is_zero()
            || c.max_request_bytes == 0
            || c.max_replace_bytes == 0
            || c.max_request_bytes > Limits::default().chunk_size as usize
            || c.max_replace_bytes > Limits::default().max_file_size as usize
            || c.deployment_physical_bytes_limit <= 0
            || c.deployment_staging_bytes_limit <= 0
        {
            return Err(invalid("LIMITS"));
        }
        c.backend_probe_deadline =
            Duration::from_millis(parse("BACKEND_PROBE_DEADLINE_MS", 1000u64)?);
        if !(Duration::from_millis(100)..=Duration::from_secs(1))
            .contains(&c.backend_probe_deadline)
        {
            return Err(invalid("LIMITS"));
        }
        c.s3.checksum_mode = match value("S3_CHECKSUM_MODE").as_deref().unwrap_or("sha256") {
            "sha256" => ChecksumMode::Sha256,
            "none" => ChecksumMode::None,
            _ => return Err(invalid("S3_CHECKSUM_MODE")),
        };
        c.s3.deployment_id = c.deployment_id.clone();
        if let Some(prefix) = value("S3_DEPLOYMENT_PREFIX")
            && prefix != c.deployment_id.as_str()
        {
            return Err(invalid("S3_DEPLOYMENT_PREFIX"));
        }
        if let Some(endpoint) = value("S3_ENDPOINT") {
            c.s3.endpoint = endpoint.parse().map_err(|_| invalid("S3_ENDPOINT"))?;
        }
        if let Some(region) = value("S3_REGION") {
            c.s3.region = region;
        }
        if let Some(bucket) = value("S3_BUCKET") {
            c.s3.bucket = bucket;
        }
        c.s3.addressing = match value("S3_ADDRESSING").as_deref().unwrap_or("path") {
            "path" => Addressing::Path,
            "virtual-host" => Addressing::VirtualHost,
            _ => return Err(invalid("S3_ADDRESSING")),
        };
        c.s3.ca_bundle_path = value("S3_CA_BUNDLE_PATH").map(Into::into);
        c.s3.allow_plaintext = parse("S3_ALLOW_PLAINTEXT", false)?;
        if c.s3.allow_plaintext && c.durability_profile != DurabilityProfile::Development {
            return Err(invalid("S3_ALLOW_PLAINTEXT"));
        }
        c.s3.request_timeout = Duration::from_millis(parse("S3_REQUEST_TIMEOUT_MS", 60000u64)?);
        c.s3.max_retries = parse("S3_MAX_RETRIES", 3)?;
        c.s3.credential_source = match value("S3_CREDENTIAL_SOURCE")
            .as_deref()
            .unwrap_or("environment")
        {
            "environment" => CredentialSource::Environment,
            "chain" => CredentialSource::Chain,
            "static" => CredentialSource::Static {
                access_key_id: value("S3_ACCESS_KEY_ID")
                    .ok_or_else(|| invalid("S3_ACCESS_KEY_ID"))?,
                secret_access_key: Secret::new(
                    value("S3_SECRET_ACCESS_KEY").ok_or_else(|| invalid("S3_SECRET_ACCESS_KEY"))?,
                ),
            },
            _ => return Err(invalid("S3_CREDENTIAL_SOURCE")),
        };
        c.s3.validate().map_err(|_| invalid("S3_CONFIG"))?;
        Ok(c)
    }
    /// Bootstrap needs metadata configuration only, without listener or backend credentials.
    pub fn bootstrap_from_env() -> Result<Self, ConfigError> {
        let mut c = Self::contract_defaults();
        c.database_url = Secret::new(value("DATABASE_URL").ok_or_else(|| invalid("DATABASE_URL"))?);
        c.deployment_id =
            DeploymentId::parse(value("DEPLOYMENT_ID").ok_or_else(|| invalid("DEPLOYMENT_ID"))?)
                .map_err(|_| invalid("DEPLOYMENT_ID"))?;
        c.durability_profile = match value("DURABILITY_PROFILE").as_deref().unwrap_or("durable") {
            "durable" => DurabilityProfile::Durable,
            "development" => DurabilityProfile::Development,
            _ => return Err(invalid("DURABILITY_PROFILE")),
        };
        c.db_max_connections = parse("DB_MAX_CONNECTIONS", c.db_max_connections)?;
        c.deployment_physical_bytes_limit = parse(
            "DEPLOYMENT_PHYSICAL_BYTES_LIMIT",
            c.deployment_physical_bytes_limit,
        )?;
        c.deployment_staging_bytes_limit = parse(
            "DEPLOYMENT_STAGING_BYTES_LIMIT",
            c.deployment_staging_bytes_limit,
        )?;
        if c.db_max_connections == 0
            || c.deployment_physical_bytes_limit <= 0
            || c.deployment_staging_bytes_limit <= 0
        {
            return Err(invalid("LIMITS"));
        }
        Ok(c)
    }
    pub fn pool_config(&self) -> PoolConfig {
        PoolConfig {
            url: self.database_url.clone(),
            max_connections: self.db_max_connections,
            acquire_timeout: Duration::from_secs(2),
        }
    }
    pub fn quota_config(&self) -> DeploymentQuotaConfig {
        DeploymentQuotaConfig {
            physical_bytes_limit: self.deployment_physical_bytes_limit,
            staging_bytes_limit: self.deployment_staging_bytes_limit,
        }
    }
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}
