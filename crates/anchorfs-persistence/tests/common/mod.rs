use anchorfs_core::{DeploymentId, Secret};
use anchorfs_persistence::{
    meta, migrate,
    pool::{self, PoolConfig},
};
use sqlx::PgPool;
use std::time::Duration;
use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
const POSTGRES_IMAGE_DIGEST: &str =
    "sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3";
pub struct Database {
    _container: ContainerAsync<Postgres>,
    pub pool: PgPool,
}
impl Database {
    async fn start(args: &[&str]) -> Self {
        // The module defaults to fsync=off; explicitly enable the official server's durable defaults.
        let container = Postgres::default()
            .with_fsync_enabled()
            .with_tag(format!("17@{POSTGRES_IMAGE_DIGEST}"))
            .with_cmd(args.iter().copied())
            .start()
            .await
            .expect("Docker and disposable PostgreSQL are required");
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let config = PoolConfig {
            url: Secret::new(format!(
                "postgres://postgres:postgres@{host}:{port}/postgres"
            )),
            max_connections: 4,
            acquire_timeout: Duration::from_secs(15),
        };
        let pool = pool::connect(&config).await.unwrap();
        Self {
            _container: container,
            pool,
        }
    }
    pub async fn migrated() -> Self {
        let db = Self::start(&[]).await;
        migrate::apply(&db.pool).await.unwrap();
        meta::initialize(
            &db.pool,
            &DeploymentId::parse("test-deployment").unwrap(),
            &quotas(),
            anchorfs_core::Limits::default().chunk_size,
        )
        .await
        .unwrap();
        db
    }
}
fn quotas() -> meta::DeploymentQuotaConfig {
    meta::DeploymentQuotaConfig {
        physical_bytes_limit: 1_000_000,
        staging_bytes_limit: 100_000,
    }
}
