//! Metadata-only truncation inside the caller's receipt transaction.
use super::{common, runner::unavailable};
use crate::error::ApiError;
use anchorfs_core::{
    ChunkGeometry, ErrorCode, HandleId, ManifestId, OperationId, WorkspaceId, plan_truncate,
};
use anchorfs_persistence::{
    authority::ActorSession,
    handles, manifests,
    namespace::{self, InodeRow},
    quota,
};
use sqlx::{Postgres, Transaction};

pub async fn truncate_in(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: HandleId,
    operation: OperationId,
    geometry: ChunkGeometry,
    size: u64,
) -> Result<InodeRow, ApiError> {
    let handle = handles::load_for_write(tx, workspace, session, id, None).await?;
    let inode = namespace::lock_inodes(tx, workspace, &[handle.inode_id])
        .await?
        .pop()
        .ok_or_else(unavailable)?;
    let handle = handles::load_for_write(tx, workspace, session, id, None).await?;
    let manifest = manifests::current(tx, workspace, &inode).await?;
    if manifest.chunk_size as u32 != geometry.chunk_size {
        return Err(ApiError::new(ErrorCode::Unsupported, ""));
    }
    let rows = manifests::chunks(tx, workspace, ManifestId(manifest.id), None).await?;
    let existing = rows
        .iter()
        .map(common::existing_chunk)
        .collect::<Result<Vec<_>, _>>()?;
    let plan = plan_truncate(geometry, inode.size as u64, size, &existing)?;
    let (_, next) =
        manifests::publish_truncate(tx, workspace, &inode, operation, size, &plan).await?;
    quota::apply(
        tx,
        workspace,
        quota::Charge {
            inodes: 0,
            bytes: next.size - inode.size,
        },
    )
    .await?;
    handles::assert_live(tx, &handle).await?;
    Ok(next)
}
