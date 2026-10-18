use anchorfs_core::WorkspaceId;
use sqlx::{Postgres, Transaction};
/// Caller must hold the inode FOR UPDATE before making a release decision.
pub async fn open_unexpired_count(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM handles WHERE workspace_id=$1 AND inode_id=$2 AND closed_at IS NULL AND expires_at>clock_timestamp()").bind(workspace.0).bind(inode).fetch_one(&mut **tx).await
}
