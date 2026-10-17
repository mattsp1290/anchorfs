use anchorfs_core::{DeploymentId, OBJECT_FORMAT_VERSION, SCHEMA_VERSION};
use sqlx::PgPool;

#[derive(Debug, Clone, Copy)]
pub struct DeploymentQuotaConfig {
    pub physical_bytes_limit: i64,
    pub staging_bytes_limit: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct Versions {
    pub schema_version: i32,
    pub object_format_version: i32,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("database belongs to deployment {stored}, configured deployment is {configured}")]
    DeploymentMismatch { stored: String, configured: String },
    #[error("expected exactly one schema_meta row, found {0}")]
    MetadataRowCount(usize),
}
#[derive(Debug, thiserror::Error)]
#[error("unsupported database versions: schema {schema_version}, objects {object_format_version}")]
pub struct VersionMismatch {
    pub schema_version: i32,
    pub object_format_version: i32,
}

/// Initialize deployment identity and quotas together. Existing quotas remain administrator-owned.
pub async fn initialize(
    pool: &PgPool,
    deployment: &DeploymentId,
    quotas: &DeploymentQuotaConfig,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO schema_meta (singleton,schema_version,object_format_version,deployment_id,created_at) VALUES (true,$1,$2,$3,now()) ON CONFLICT DO NOTHING")
        .bind(SCHEMA_VERSION).bind(OBJECT_FORMAT_VERSION).bind(deployment.as_str()).execute(&mut *tx).await?;
    let stored: String =
        sqlx::query_scalar("SELECT deployment_id FROM schema_meta WHERE singleton")
            .fetch_one(&mut *tx)
            .await?;
    if stored != deployment.as_str() {
        return Err(Error::DeploymentMismatch {
            stored,
            configured: deployment.to_string(),
        });
    }
    sqlx::query("INSERT INTO deployment_quotas (singleton,physical_bytes_limit,staging_bytes_limit) VALUES (true,$1,$2) ON CONFLICT DO NOTHING")
        .bind(quotas.physical_bytes_limit).bind(quotas.staging_bytes_limit).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn read_versions(pool: &PgPool) -> Result<Versions, Error> {
    let rows: Vec<Versions> =
        sqlx::query_as("SELECT schema_version,object_format_version FROM schema_meta")
            .fetch_all(pool)
            .await?;
    if rows.len() != 1 {
        return Err(Error::MetadataRowCount(rows.len()));
    }
    Ok(rows[0])
}
pub fn check_versions(versions: &Versions) -> Result<(), VersionMismatch> {
    if versions.schema_version == SCHEMA_VERSION
        && versions.object_format_version == OBJECT_FORMAT_VERSION
    {
        Ok(())
    } else {
        Err(VersionMismatch {
            schema_version: versions.schema_version,
            object_format_version: versions.object_format_version,
        })
    }
}
