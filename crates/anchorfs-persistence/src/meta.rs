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
    #[error("invalid deployment chunk size {0}")]
    InvalidChunkSize(u32),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("database belongs to deployment {stored}, configured deployment is {configured}")]
    DeploymentMismatch { stored: String, configured: String },
    #[error("database chunk size {stored} differs from configured chunk size {configured}")]
    ChunkSizeMismatch { stored: i32, configured: u32 },
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
    chunk_size: u32,
) -> Result<(), Error> {
    if !(4096..=anchorfs_core::Limits::default().chunk_size).contains(&chunk_size) {
        return Err(Error::InvalidChunkSize(chunk_size));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO schema_meta (singleton,schema_version,object_format_version,deployment_id,created_at,chunk_size) VALUES (true,$1,$2,$3,now(),$4) ON CONFLICT DO NOTHING")
        .bind(SCHEMA_VERSION).bind(OBJECT_FORMAT_VERSION).bind(deployment.as_str()).bind(chunk_size as i32).execute(&mut *tx).await?;
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
    let stored: i32 = sqlx::query_scalar("SELECT chunk_size FROM schema_meta WHERE singleton")
        .fetch_one(&mut *tx)
        .await?;
    if i64::from(stored) != i64::from(chunk_size) {
        return Err(Error::ChunkSizeMismatch {
            stored,
            configured: chunk_size,
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
