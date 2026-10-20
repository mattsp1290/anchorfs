//! Logical charges are changed only after locking every affected inode.
use crate::namespace::InodeRow;
use crate::reservations::{QuotaKind, ReservationError};
use anchorfs_core::WorkspaceId;
use sqlx::{Postgres, Transaction};

#[derive(sqlx::FromRow)]
struct StorageQuota {
    staging_bytes_reserved: i64,
    staging_bytes_limit: i64,
    physical_bytes_used: i64,
    physical_bytes_limit: i64,
}
fn check_storage(
    quota: &StorageQuota,
    bytes: i64,
    staging: QuotaKind,
    physical: QuotaKind,
) -> Result<(), ReservationError> {
    if bytes <= 0 {
        return Err(sqlx::Error::Protocol("reservation charge must be positive".into()).into());
    }
    let reserved = i128::from(quota.staging_bytes_reserved) + i128::from(bytes);
    if reserved > i128::from(quota.staging_bytes_limit) {
        return Err(ReservationError::Exceeded(staging));
    }
    if reserved + i128::from(quota.physical_bytes_used) > i128::from(quota.physical_bytes_limit) {
        return Err(ReservationError::Exceeded(physical));
    }
    Ok(())
}

/// Workspace then deployment locks are acquired last in byte transactions.
/// Any error requires rolling back the caller's transaction or savepoint.
pub async fn reserve_staging(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    bytes: i64,
) -> Result<(), ReservationError> {
    let w = sqlx::query_as::<_, StorageQuota>(
        "SELECT * FROM workspace_quotas WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace.0)
    .fetch_one(&mut **tx)
    .await?;
    check_storage(
        &w,
        bytes,
        QuotaKind::WorkspaceStaging,
        QuotaKind::WorkspacePhysical,
    )?;
    sqlx::query("UPDATE workspace_quotas SET staging_bytes_reserved=staging_bytes_reserved+$2 WHERE workspace_id=$1")
        .bind(workspace.0).bind(bytes).execute(&mut **tx).await?;
    let d = sqlx::query_as::<_, StorageQuota>(
        "SELECT * FROM deployment_quotas WHERE singleton FOR UPDATE",
    )
    .fetch_one(&mut **tx)
    .await?;
    check_storage(
        &d,
        bytes,
        QuotaKind::DeploymentStaging,
        QuotaKind::DeploymentPhysical,
    )?;
    sqlx::query("UPDATE deployment_quotas SET staging_bytes_reserved=staging_bytes_reserved+$1 WHERE singleton")
        .bind(bytes).execute(&mut **tx).await?;
    Ok(())
}

pub async fn settle(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    reserved: i64,
    published: i64,
    logical_delta: i64,
) -> Result<(), QuotaError> {
    // apply takes the workspace quota lock before the deployment lock below.
    apply(
        tx,
        workspace,
        Charge {
            inodes: 0,
            bytes: logical_delta,
        },
    )
    .await?;
    release_to_orphans(tx, workspace, reserved, published).await?;
    Ok(())
}

/// Conservative physical charges may exceed limits. Never drop them on expiry.
pub async fn release_to_orphans(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    reserved: i64,
    orphan_bytes: i64,
) -> Result<(), sqlx::Error> {
    if reserved < 0 || orphan_bytes < 0 {
        return Err(sqlx::Error::Protocol("negative storage settlement".into()));
    }
    let w = sqlx::query("UPDATE workspace_quotas SET staging_bytes_reserved=staging_bytes_reserved-$2,physical_bytes_used=physical_bytes_used+$3 WHERE workspace_id=$1")
        .bind(workspace.0).bind(reserved).bind(orphan_bytes).execute(&mut **tx).await?;
    if w.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    let d = sqlx::query("UPDATE deployment_quotas SET staging_bytes_reserved=staging_bytes_reserved-$1,physical_bytes_used=physical_bytes_used+$2 WHERE singleton")
        .bind(reserved).bind(orphan_bytes).execute(&mut **tx).await?;
    if d.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}
#[derive(Debug, Clone, Copy)]
pub struct Charge {
    pub inodes: i64,
    pub bytes: i64,
}
#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("quota_exceeded")]
    Exceeded,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
pub async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    charge: Charge,
) -> Result<(), QuotaError> {
    if charge.inodes == 0 && charge.bytes == 0 {
        return Ok(());
    }
    let result = sqlx::query("UPDATE workspace_quotas SET logical_inodes_used=logical_inodes_used+$2,logical_bytes_used=logical_bytes_used+$3 WHERE workspace_id=$1 AND ($2<=0 OR logical_inodes_used+$2<=logical_inodes_limit) AND ($3<=0 OR logical_bytes_used+$3<=logical_bytes_limit)").bind(workspace.0).bind(charge.inodes).bind(charge.bytes).execute(&mut **tx).await?;
    if result.rows_affected() == 0 {
        return Err(QuotaError::Exceeded);
    }
    Ok(())
}
/// The caller holds the inode FOR UPDATE. Re-read after acquiring that lock.
pub async fn release_if_due(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
) -> Result<bool, QuotaError> {
    if inode.nlink != 0
        || inode.charge_released_at.is_some()
        || crate::handles::open_unexpired_count(tx, workspace, inode.id).await? != 0
    {
        return Ok(false);
    }
    let updated = sqlx::query("UPDATE inodes SET charge_released_at=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 AND charge_released_at IS NULL").bind(workspace.0).bind(inode.id).execute(&mut **tx).await?;
    if updated.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    apply(
        tx,
        workspace,
        Charge {
            inodes: -1,
            bytes: -inode.size,
        },
    )
    .await?;
    Ok(true)
}
