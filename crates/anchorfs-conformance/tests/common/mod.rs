#![allow(dead_code)]
pub mod byte_backend;
pub mod client;
pub mod fake_backend;
pub mod fixture;
pub mod minio;
pub mod proxy;
use anchorfs_core::{DeploymentId, Secret};
use anchorfs_persistence::{
    meta::{self, DeploymentQuotaConfig},
    migrate,
    pool::{self, PgPool, PoolConfig},
};
use anchorfs_service::{app::AppState, config::Config};
use std::{sync::Arc, time::Duration};
use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
pub const POSTGRES_IMAGE_DIGEST: &str =
    "sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3";
pub async fn database(settings: &[&str]) -> (ContainerAsync<Postgres>, PgPool, String) {
    database_with(
        settings,
        &DeploymentId::parse("test").unwrap(),
        anchorfs_core::Limits::default().chunk_size,
        4,
    )
    .await
}
pub async fn database_with(
    settings: &[&str],
    deployment: &DeploymentId,
    chunk_size: u32,
    max_connections: u32,
) -> (ContainerAsync<Postgres>, PgPool, String) {
    let mut command = vec!["postgres".to_owned()];
    for setting in settings {
        command.extend(["-c".into(), setting.to_string()]);
    }
    let container = Postgres::default()
        .with_fsync_enabled()
        .with_tag(format!("17@{POSTGRES_IMAGE_DIGEST}"))
        .with_cmd(command)
        .start()
        .await
        .unwrap();
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap()
    );
    let pool = pool::connect(&PoolConfig {
        url: Secret::new(url.clone()),
        max_connections,
        acquire_timeout: Duration::from_secs(2),
    })
    .await
    .unwrap();
    migrate::apply(&pool).await.unwrap();
    meta::initialize(
        &pool,
        deployment,
        &DeploymentQuotaConfig {
            physical_bytes_limit: 1 << 40,
            staging_bytes_limit: 1 << 30,
        },
        chunk_size,
    )
    .await
    .unwrap();
    (container, pool, url)
}
pub fn unreachable_s3() -> anchorfs_backend::config::S3Config {
    let mut s3 = Config::contract_defaults().s3;
    s3.endpoint = "http://127.0.0.1:1".parse().unwrap();
    s3.allow_plaintext = true;
    s3.request_timeout = Duration::from_secs(1);
    s3.max_retries = 0;
    s3
}
pub fn minio_s3(minio: &minio::Minio) -> anchorfs_backend::config::S3Config {
    minio::backend_config(
        &minio.endpoint,
        DeploymentId::parse("test").unwrap(),
        anchorfs_backend::config::ChecksumMode::Sha256,
        Duration::from_secs(20),
        2,
    )
}
pub fn state(pool: PgPool) -> AppState {
    state_with(pool, unreachable_s3())
}
pub fn state_with(pool: PgPool, s3: anchorfs_backend::config::S3Config) -> AppState {
    let mut config = Config::contract_defaults();
    config.deployment_id = s3.deployment_id.clone();
    config.s3 = s3;
    AppState {
        cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
            .expect("validated cursor signing key"),
        control_permits: std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.db_max_connections.saturating_sub(2).max(1) as usize,
        )),
        pool,
        backend: Arc::new(anchorfs_backend::s3::S3Backend::from_config(&config.s3).unwrap()),
        probe_cache: Default::default(),
        data_permits: Arc::new(tokio::sync::Semaphore::new(6)),
        faults: Arc::new(anchorfs_service::pipeline::faults::NoFaults),
        config: Arc::new(config),
    }
}
pub fn unavailable() -> AppState {
    state(PgPool::connect_lazy("postgres://unused:unused@127.0.0.1:1/unused").unwrap())
}

/// Counting in-memory stub; control-plane tests assert every backend entry remains unused.
pub fn state_fake(pool: PgPool) -> (AppState, Arc<fake_backend::FakeBackend>) {
    let backend = Arc::new(fake_backend::FakeBackend::default());
    let state = AppState {
        cursor_key: anchorfs_core::decode_hex_32(
            Config::contract_defaults().cursor_signing_key.expose(),
        )
        .unwrap(),
        pool,
        config: Arc::new(Config::contract_defaults()),
        backend: backend.clone(),
        probe_cache: Default::default(),
        data_permits: Arc::new(tokio::sync::Semaphore::new(6)),
        faults: Arc::new(anchorfs_service::pipeline::faults::NoFaults),
        control_permits: Arc::new(tokio::sync::Semaphore::new(2)),
    };
    (state, backend)
}
pub mod api_proxy;

pub mod tls;
