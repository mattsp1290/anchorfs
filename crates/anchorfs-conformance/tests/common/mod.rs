#![allow(dead_code)]
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
        max_connections: 4,
        acquire_timeout: Duration::from_secs(2),
    })
    .await
    .unwrap();
    migrate::apply(&pool).await.unwrap();
    meta::initialize(
        &pool,
        &DeploymentId::parse("test").unwrap(),
        &DeploymentQuotaConfig {
            physical_bytes_limit: 1 << 40,
            staging_bytes_limit: 1 << 30,
        },
    )
    .await
    .unwrap();
    (container, pool, url)
}
pub fn state(pool: PgPool) -> AppState {
    let config = Config::contract_defaults();
    AppState {
        pool,
        config: Arc::new(config),
    }
}
pub fn unavailable() -> AppState {
    state(PgPool::connect_lazy("postgres://unused:unused@127.0.0.1:1/unused").unwrap())
}
