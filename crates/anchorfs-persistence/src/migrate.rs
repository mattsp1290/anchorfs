use sqlx::{
    PgPool,
    migrate::{MigrateError, Migrator},
};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn apply(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}
pub async fn revert_all(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.undo(pool, 0).await
}

/// A newly provisioned database has no migration history and is not newer.
pub async fn database_is_newer(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    if !exists {
        return Ok(false);
    }
    let maximum = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success AND version > $1)",
    )
    .bind(maximum)
    .fetch_one(pool)
    .await
}
