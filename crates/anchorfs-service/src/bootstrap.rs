//! Operator-only bootstrap; credentials never enter configuration or tracing fields.
use anchorfs_persistence::{
    credentials::{self, BootstrapError, Issued},
    pool::PgPool,
};
use time::OffsetDateTime;

pub async fn create_administrator(
    pool: &PgPool,
    uid: u32,
    gid: u32,
    max_reservation_bytes: i64,
    expires_at: Option<OffsetDateTime>,
) -> Result<Issued, BootstrapError> {
    let mut tx = pool.begin().await?;
    let issued =
        credentials::bootstrap_administrator(&mut tx, uid, gid, max_reservation_bytes, expires_at)
            .await?;
    tx.commit().await?;
    Ok(issued)
}
