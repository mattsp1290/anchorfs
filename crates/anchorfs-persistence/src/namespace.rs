//! Independently authored namespace operations. Paths never follow symlinks.
use anchorfs_core::{Access, Identity, Mode, Name, WorkspaceId, WorkspacePath, permits};
use sqlx::{Postgres, Transaction};
use std::sync::atomic::{AtomicBool, Ordering};
use time::OffsetDateTime;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InodeRow {
    pub workspace_id: uuid::Uuid,
    pub id: i64,
    pub generation: i64,
    pub kind: String,
    pub size: i64,
    pub mode: i32,
    pub uid: i64,
    pub gid: i64,
    pub atime: OffsetDateTime,
    pub mtime: OffsetDateTime,
    pub ctime: OffsetDateTime,
    pub nlink: i32,
    pub current_manifest_id: Option<uuid::Uuid>,
    pub symlink_target: Option<String>,
    pub unlinked_at: Option<OffsetDateTime>,
    pub charge_released_at: Option<OffsetDateTime>,
}
#[derive(Debug, Clone, Copy)]
pub struct Caller {
    pub identity: Identity,
    pub admin: bool,
}
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    #[error("not_found")]
    NotFound,
    #[error("not_directory")]
    NotDirectory,
    #[error("is_directory")]
    IsDirectory,
    #[error("directory_not_empty")]
    DirectoryNotEmpty,
    #[error("exists")]
    Exists,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid_input")]
    InvalidInput,
    #[error(transparent)]
    Quota(#[from] crate::quota::QuotaError),
    #[error("not_implemented")]
    NotImplemented,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
/// Constructed only after locking the workspace's namespace row.
pub struct NamespaceGuard {
    workspace: WorkspaceId,
    generation: i64,
    bumped: AtomicBool,
}
pub async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
) -> Result<NamespaceGuard, sqlx::Error> {
    let generation = sqlx::query_scalar(
        "SELECT namespace_generation FROM namespace_locks WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace.0)
    .fetch_one(&mut **tx)
    .await?;
    Ok(NamespaceGuard {
        workspace,
        generation,
        bumped: AtomicBool::new(false),
    })
}
impl NamespaceGuard {
    pub fn generation(&self) -> i64 {
        self.generation + i64::from(self.bumped.load(Ordering::Relaxed))
    }
    pub async fn bump(&self, tx: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
        if !self.bumped.load(Ordering::Relaxed) {
            sqlx::query("UPDATE namespace_locks SET namespace_generation=namespace_generation+1 WHERE workspace_id=$1").bind(self.workspace.0).execute(&mut **tx).await?;
            self.bumped.store(true, Ordering::Relaxed);
        }
        Ok(self.generation())
    }
}
pub async fn current_generation(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT namespace_generation FROM namespace_locks WHERE workspace_id=$1")
        .bind(workspace.0)
        .fetch_one(&mut **tx)
        .await
}
pub async fn root(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
) -> Result<InodeRow, sqlx::Error> {
    sqlx::query_as("SELECT i.* FROM inodes i JOIN workspaces w ON w.id=i.workspace_id AND w.root_inode_id=i.id WHERE w.id=$1").bind(workspace.0).fetch_one(&mut **tx).await
}
pub async fn child(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    parent: i64,
    name: &str,
) -> Result<Option<InodeRow>, sqlx::Error> {
    sqlx::query_as("SELECT i.* FROM inodes i JOIN directory_entries e ON e.workspace_id=i.workspace_id AND e.inode_id=i.id WHERE e.workspace_id=$1 AND e.parent_inode_id=$2 AND e.name=$3").bind(workspace.0).bind(parent).bind(name).fetch_optional(&mut **tx).await
}
pub fn access(row: &InodeRow, caller: &Caller, want: Access) -> Result<(), NamespaceError> {
    if caller.admin
        || permits(
            Mode(row.mode as u32),
            row.uid as u32,
            row.gid as u32,
            caller.identity,
            want,
        )
    {
        Ok(())
    } else {
        Err(NamespaceError::Forbidden)
    }
}
pub fn parent_access(row: &InodeRow, caller: &Caller) -> Result<(), NamespaceError> {
    if row.nlink == 0 {
        return Err(NamespaceError::NotFound);
    }
    if row.kind != "directory" {
        return Err(NamespaceError::NotDirectory);
    }
    access(
        row,
        caller,
        Access {
            write: true,
            exec: true,
            ..Access::default()
        },
    )
}
pub async fn resolve(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    path: Option<&WorkspacePath>,
    caller: &Caller,
) -> Result<InodeRow, NamespaceError> {
    let mut current = root(tx, workspace).await?;
    if let Some(path) = path {
        for component in path.components() {
            if current.kind != "directory" {
                return Err(NamespaceError::NotDirectory);
            }
            access(
                &current,
                caller,
                Access {
                    exec: true,
                    ..Access::default()
                },
            )?;
            current = child(tx, workspace, current.id, component)
                .await?
                .ok_or(NamespaceError::NotFound)?;
        }
    }
    Ok(current)
}
pub async fn resolve_parent(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    path: &WorkspacePath,
    caller: &Caller,
) -> Result<(InodeRow, Name), NamespaceError> {
    let (parent, name) = path.split_parent();
    let row = resolve(tx, workspace, parent.as_ref(), caller).await?;
    parent_access(&row, caller)?;
    Ok((row, name))
}
pub async fn lock_inodes(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    ids: &[i64],
) -> Result<Vec<InodeRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM inodes WHERE workspace_id=$1 AND id=ANY($2) ORDER BY id FOR UPDATE",
    )
    .bind(workspace.0)
    .bind(ids)
    .fetch_all(&mut **tx)
    .await
}
pub async fn ancestors(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    // UNION deduplicates even a corrupt cycle, so diagnostics cannot hang.
    sqlx::query_scalar("WITH RECURSIVE ancestors(id) AS (SELECT $2::bigint UNION SELECT e.parent_inode_id FROM directory_entries e JOIN ancestors a ON e.inode_id=a.id WHERE e.workspace_id=$1) SELECT id FROM ancestors").bind(workspace.0).bind(inode).fetch_all(&mut **tx).await
}
