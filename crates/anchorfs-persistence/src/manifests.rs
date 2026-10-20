//! Immutable manifest publication. Callers hold the inode FOR UPDATE and publish
//! objects, quotas and receipts in this same transaction before committing.
use crate::namespace::InodeRow;
use anchorfs_core::{ManifestId, OperationId, WorkspaceId};
use anchorfs_core::{ObjectId, Sha256Digest, TruncatePlan};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ManifestRow {
    pub workspace_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub inode_id: i64,
    pub size: i64,
    pub chunk_size: i32,
    pub generation: i64,
    pub created_by_operation_id: uuid::Uuid,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChunkRow {
    pub chunk_index: i32,
    pub chunk_size: i32,
    pub object_id: uuid::Uuid,
    pub sha256: Vec<u8>,
    pub logical_length: i32,
    pub object_length: i64,
}

#[derive(Debug, Clone)]
pub struct NewChunk {
    pub index: u32,
    pub object: ObjectId,
    pub sha256: Sha256Digest,
    pub logical_length: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("missing file manifest")]
    NotFound,
    #[error("manifest chunk size differs from deployment geometry")]
    ChunkSizeMismatch,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    manifest: ManifestId,
) -> Result<ManifestRow, ManifestError> {
    sqlx::query_as("SELECT * FROM manifests WHERE workspace_id=$1 AND id=$2")
        .bind(workspace.0)
        .bind(manifest.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ManifestError::NotFound)
}

pub async fn current(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
) -> Result<ManifestRow, ManifestError> {
    load(
        tx,
        workspace,
        ManifestId(inode.current_manifest_id.ok_or(ManifestError::NotFound)?),
    )
    .await
}

pub async fn chunks(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    manifest: ManifestId,
    indices: Option<&[i32]>,
) -> Result<Vec<ChunkRow>, sqlx::Error> {
    sqlx::query_as("SELECT c.chunk_index,c.chunk_size,c.object_id,c.sha256,c.logical_length,o.byte_length AS object_length FROM manifest_chunks c JOIN objects o ON (o.workspace_id,o.id,o.sha256)=(c.workspace_id,c.object_id,c.sha256) WHERE c.workspace_id=$1 AND c.manifest_id=$2 AND ($3::integer[] IS NULL OR c.chunk_index=ANY($3)) ORDER BY c.chunk_index")
        .bind(workspace.0).bind(manifest.0).bind(indices).fetch_all(&mut **tx).await
}

struct Publication<'a> {
    inode: &'a InodeRow,
    operation: OperationId,
    chunk_size: u32,
    size: u64,
    bump: bool,
}
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    p: &Publication<'_>,
) -> Result<ManifestRow, ManifestError> {
    let size = i64::try_from(p.size)
        .map_err(|_| sqlx::Error::Protocol("manifest size overflow".into()))?;
    let chunk_size = i32::try_from(p.chunk_size).map_err(|_| ManifestError::ChunkSizeMismatch)?;
    let generation = p
        .inode
        .generation
        .checked_add(i64::from(p.bump))
        .ok_or_else(|| sqlx::Error::Protocol("manifest generation overflow".into()))?;
    Ok(sqlx::query_as("INSERT INTO manifests(workspace_id,id,inode_id,size,chunk_size,generation,created_by_operation_id,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,transaction_timestamp()) RETURNING *")
        .bind(workspace.0).bind(ManifestId::new().0).bind(p.inode.id).bind(size)
        .bind(chunk_size).bind(generation).bind(p.operation.0).fetch_one(&mut **tx).await?)
}
async fn add_chunks(
    tx: &mut Transaction<'_, Postgres>,
    manifest: &ManifestRow,
    chunks: &[NewChunk],
) -> Result<(), sqlx::Error> {
    for chunk in chunks {
        let index = i32::try_from(chunk.index)
            .map_err(|_| sqlx::Error::Protocol("chunk index overflow".into()))?;
        let length = i32::try_from(chunk.logical_length)
            .map_err(|_| sqlx::Error::Protocol("chunk length overflow".into()))?;
        sqlx::query("INSERT INTO manifest_chunks(workspace_id,manifest_id,chunk_index,chunk_size,object_id,sha256,logical_length) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(manifest.workspace_id).bind(manifest.id).bind(index).bind(manifest.chunk_size)
            .bind(chunk.object.0).bind(chunk.sha256.0.as_slice()).bind(length).execute(&mut **tx).await?;
    }
    Ok(())
}
async fn point_inode(
    tx: &mut Transaction<'_, Postgres>,
    manifest: &ManifestRow,
    bump: bool,
) -> Result<InodeRow, sqlx::Error> {
    sqlx::query_as("UPDATE inodes SET current_manifest_id=$3,size=$4,generation=$5,mtime=CASE WHEN $6 THEN transaction_timestamp() ELSE mtime END,ctime=CASE WHEN $6 THEN transaction_timestamp() ELSE ctime END WHERE workspace_id=$1 AND id=$2 RETURNING *")
        .bind(manifest.workspace_id).bind(manifest.inode_id).bind(manifest.id).bind(manifest.size)
        .bind(manifest.generation).bind(bump).fetch_one(&mut **tx).await
}

pub async fn publish_write(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
    operation: OperationId,
    chunk_size: u32,
    new_size: u64,
    touched: &[NewChunk],
) -> Result<(ManifestRow, InodeRow), ManifestError> {
    let old = current(tx, workspace, inode).await?;
    if i64::from(old.chunk_size) != i64::from(chunk_size) {
        return Err(ManifestError::ChunkSizeMismatch);
    }
    let manifest = insert(
        tx,
        workspace,
        &Publication {
            inode,
            operation,
            chunk_size,
            size: new_size,
            bump: true,
        },
    )
    .await?;
    let indices: Vec<i64> = touched.iter().map(|chunk| i64::from(chunk.index)).collect();
    sqlx::query("INSERT INTO manifest_chunks SELECT workspace_id,$3,chunk_index,chunk_size,object_id,sha256,logical_length FROM manifest_chunks WHERE workspace_id=$1 AND manifest_id=$2 AND chunk_index::bigint<>ALL($4::bigint[])")
        .bind(workspace.0).bind(old.id).bind(manifest.id).bind(&indices).execute(&mut **tx).await?;
    add_chunks(tx, &manifest, touched).await?;
    let inode = point_inode(tx, &manifest, true).await?;
    Ok((manifest, inode))
}

pub async fn publish_truncate(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
    operation: OperationId,
    new_size: u64,
    plan: &TruncatePlan,
) -> Result<(ManifestRow, InodeRow), ManifestError> {
    let old = current(tx, workspace, inode).await?;
    if !plan.changed && i64::try_from(new_size).ok() == Some(inode.size) {
        return Ok((old, inode.clone()));
    }
    let manifest = insert(
        tx,
        workspace,
        &Publication {
            inode,
            operation,
            chunk_size: old.chunk_size as u32,
            size: new_size,
            bump: true,
        },
    )
    .await?;
    for row in &plan.kept {
        let affected = sqlx::query("INSERT INTO manifest_chunks SELECT workspace_id,$3,chunk_index,chunk_size,object_id,sha256,$5 FROM manifest_chunks WHERE workspace_id=$1 AND manifest_id=$2 AND chunk_index::bigint=$4")
            .bind(workspace.0).bind(old.id).bind(manifest.id).bind(i64::from(row.index))
            .bind(row.logical_length as i32).execute(&mut **tx).await?.rows_affected();
        if affected != 1 {
            return Err(ManifestError::NotFound);
        }
    }
    let inode = point_inode(tx, &manifest, true).await?;
    Ok((manifest, inode))
}

pub async fn publish_replace(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
    operation: OperationId,
    chunk_size: u32,
    size: u64,
    chunks: &[NewChunk],
) -> Result<(ManifestRow, InodeRow), ManifestError> {
    let old = current(tx, workspace, inode).await?;
    if i64::from(old.chunk_size) != i64::from(chunk_size) {
        return Err(ManifestError::ChunkSizeMismatch);
    }
    let manifest = insert(
        tx,
        workspace,
        &Publication {
            inode,
            operation,
            chunk_size,
            size,
            bump: true,
        },
    )
    .await?;
    add_chunks(tx, &manifest, chunks).await?;
    let inode = point_inode(tx, &manifest, true).await?;
    Ok((manifest, inode))
}

pub async fn publish_empty(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: &InodeRow,
    operation: OperationId,
    chunk_size: u32,
    bump: bool,
) -> Result<(ManifestRow, InodeRow), ManifestError> {
    if bump {
        let old = current(tx, workspace, inode).await?;
        if i64::from(old.chunk_size) != i64::from(chunk_size) {
            return Err(ManifestError::ChunkSizeMismatch);
        }
    }
    let manifest = insert(
        tx,
        workspace,
        &Publication {
            inode,
            operation,
            chunk_size,
            size: 0,
            bump,
        },
    )
    .await?;
    let inode = point_inode(tx, &manifest, bump).await?;
    Ok((manifest, inode))
}
