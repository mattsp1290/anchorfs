//! Atomic creation of a workspace and its empty root namespace.
use anchorfs_core::WorkspaceId;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

pub struct NewWorkspace {
    pub logical_bytes_limit: i64,
    pub logical_inodes_limit: i64,
    pub physical_bytes_limit: i64,
    pub staging_bytes_limit: i64,
    pub root_uid: u32,
    pub root_gid: u32,
    pub root_mode: u32,
}
#[derive(Debug, sqlx::FromRow)]
pub struct WorkspaceView {
    pub id: uuid::Uuid,
    pub authorization_epoch: i64,
    pub mutation_epoch: i64,
    pub inode_id: i64,
    pub generation: i64,
    pub kind: String,
    pub size: i64,
    pub mode: i32,
    pub uid: i64,
    pub gid: i64,
    pub nlink: i32,
    pub atime: OffsetDateTime,
    pub mtime: OffsetDateTime,
    pub ctime: OffsetDateTime,
}
pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    new: NewWorkspace,
) -> Result<WorkspaceView, sqlx::Error> {
    let id = WorkspaceId::new();
    let root: i64 = sqlx::query_scalar("SELECT nextval('inodes_id_seq')")
        .fetch_one(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO workspaces (id,root_inode_id,state,authorization_epoch,mutation_epoch,created_at,updated_at) VALUES ($1,$2,'active',1,1,transaction_timestamp(),transaction_timestamp())")
        .bind(id.0).bind(root).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO inodes (workspace_id,id,generation,kind,size,mode,uid,gid,nlink,atime,mtime,ctime) VALUES ($1,$2,1,'directory',0,$3,$4,$5,2,transaction_timestamp(),transaction_timestamp(),transaction_timestamp())")
        .bind(id.0).bind(root).bind(new.root_mode as i32).bind(i64::from(new.root_uid)).bind(i64::from(new.root_gid)).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO workspace_quotas (workspace_id,logical_bytes_limit,logical_inodes_limit,physical_bytes_limit,staging_bytes_limit) VALUES ($1,$2,$3,$4,$5)")
        .bind(id.0).bind(new.logical_bytes_limit).bind(new.logical_inodes_limit).bind(new.physical_bytes_limit).bind(new.staging_bytes_limit).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO namespace_locks (workspace_id) VALUES ($1)")
        .bind(id.0)
        .execute(&mut **tx)
        .await?;
    read(tx, id).await?.ok_or(sqlx::Error::RowNotFound)
}
pub async fn read(
    tx: &mut Transaction<'_, Postgres>,
    id: WorkspaceId,
) -> Result<Option<WorkspaceView>, sqlx::Error> {
    sqlx::query_as("SELECT w.id,w.authorization_epoch,w.mutation_epoch,i.id AS inode_id,i.generation,i.kind,i.size,i.mode,i.uid,i.gid,i.nlink,i.atime,i.mtime,i.ctime FROM workspaces w JOIN inodes i ON i.workspace_id=w.id AND i.id=w.root_inode_id WHERE w.id=$1")
        .bind(id.0).fetch_optional(&mut **tx).await
}
