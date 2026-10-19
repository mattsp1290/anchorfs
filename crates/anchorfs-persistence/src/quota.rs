//! Logical charges are changed only after locking every affected inode.
use crate::namespace::InodeRow;
use anchorfs_core::WorkspaceId;
use sqlx::{Postgres, Transaction};
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
