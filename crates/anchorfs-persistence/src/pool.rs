use anchorfs_core::Secret;
pub use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub url: Secret<String>,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

pub async fn connect(config: &PoolConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET synchronous_commit = on")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(config.url.expose())
        .await
}
