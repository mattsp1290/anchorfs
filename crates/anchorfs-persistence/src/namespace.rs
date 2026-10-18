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

#[derive(Debug)]
pub struct Mutated {
    pub inode_id: i64,
    pub generation: i64,
}
fn mutated(row: &InodeRow) -> Mutated {
    Mutated {
        inode_id: row.id,
        generation: row.generation,
    }
}
fn locked(rows: &[InodeRow], id: i64) -> Result<&InodeRow, NamespaceError> {
    rows.iter()
        .find(|r| r.id == id && r.nlink > 0)
        .ok_or(NamespaceError::NotFound)
}
pub fn validate_mode(mode: u32) -> Result<(), NamespaceError> {
    if mode > 0o777 {
        Err(NamespaceError::InvalidInput)
    } else {
        Ok(())
    }
}
async fn unchanged(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    parent: i64,
    name: &str,
    expected: Option<i64>,
) -> Result<(), NamespaceError> {
    if child(tx, workspace, parent, name).await?.map(|r| r.id) == expected {
        Ok(())
    } else {
        Err(NamespaceError::NotFound)
    }
}
async fn touch_parent(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    id: i64,
    nlink_delta: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE inodes SET generation=generation+1,nlink=nlink+$3,mtime=transaction_timestamp(),ctime=transaction_timestamp() WHERE workspace_id=$1 AND id=$2").bind(workspace.0).bind(id).bind(nlink_delta).execute(&mut **tx).await?;
    Ok(())
}
async fn insert_entry(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    parent: i64,
    name: &str,
    inode: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO directory_entries(workspace_id,parent_inode_id,name,inode_id) VALUES ($1,$2,$3,$4)").bind(workspace.0).bind(parent).bind(name).bind(inode).execute(&mut **tx).await?;
    Ok(())
}
async fn delete_entry(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    parent: i64,
    name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM directory_entries WHERE workspace_id=$1 AND parent_inode_id=$2 AND name=$3",
    )
    .bind(workspace.0)
    .bind(parent)
    .bind(name)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub async fn empty(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    inode: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM directory_entries WHERE workspace_id=$1 AND parent_inode_id=$2)").bind(workspace.0).bind(inode).fetch_one(&mut **tx).await
}
/// Shared create path also used by metadata-only open(create).
pub async fn create_file(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
    mode: u32,
) -> Result<InodeRow, NamespaceError> {
    create(tx, guard, caller, path, mode, "file", None).await
}
async fn create(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
    mode: u32,
    kind: &str,
    target: Option<&str>,
) -> Result<InodeRow, NamespaceError> {
    validate_mode(mode)?;
    let w = guard.workspace;
    let (parent, name) = resolve_parent(tx, w, path, caller).await?;
    let rows = lock_inodes(tx, w, &[parent.id]).await?;
    parent_access(locked(&rows, parent.id)?, caller)?;
    if child(tx, w, parent.id, name.as_str()).await?.is_some() {
        return Err(NamespaceError::Exists);
    }
    let row: InodeRow = sqlx::query_as("INSERT INTO inodes(workspace_id,kind,size,mode,uid,gid,nlink,symlink_target,atime,mtime,ctime) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,transaction_timestamp(),transaction_timestamp(),transaction_timestamp()) RETURNING *")
        .bind(w.0).bind(kind).bind(target.map_or(0,|t|t.len() as i64)).bind(mode as i32).bind(i64::from(caller.identity.uid)).bind(i64::from(caller.identity.gid)).bind(if kind=="directory" {2_i32} else {1_i32}).bind(target).fetch_one(&mut **tx).await?;
    insert_entry(tx, w, parent.id, name.as_str(), row.id).await?;
    touch_parent(tx, w, parent.id, i32::from(kind == "directory")).await?;
    crate::quota::apply(
        tx,
        w,
        crate::quota::Charge {
            inodes: 1,
            bytes: row.size,
        },
    )
    .await?;
    guard.bump(tx).await?;
    Ok(row)
}
pub async fn mkdir(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
    mode: u32,
) -> Result<Mutated, NamespaceError> {
    Ok(mutated(
        &create(tx, guard, caller, path, mode, "directory", None).await?,
    ))
}
pub async fn symlink(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
    target: &str,
) -> Result<Mutated, NamespaceError> {
    if target.is_empty() || target.len() > 4096 || target.contains('\0') {
        return Err(NamespaceError::InvalidInput);
    }
    Ok(mutated(
        &create(tx, guard, caller, path, 0o777, "symlink", Some(target)).await?,
    ))
}
pub async fn link(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    existing: &WorkspacePath,
    path: &WorkspacePath,
) -> Result<Mutated, NamespaceError> {
    let w = guard.workspace;
    let source = resolve(tx, w, Some(existing), caller).await?;
    if source.kind == "directory" {
        return Err(NamespaceError::IsDirectory);
    }
    let (parent, name) = resolve_parent(tx, w, path, caller).await?;
    let rows = lock_inodes(tx, w, &[source.id, parent.id]).await?;
    locked(&rows, source.id)?;
    parent_access(locked(&rows, parent.id)?, caller)?;
    if child(tx, w, parent.id, name.as_str()).await?.is_some() {
        return Err(NamespaceError::Exists);
    }
    insert_entry(tx, w, parent.id, name.as_str(), source.id).await?;
    let row: InodeRow = sqlx::query_as("UPDATE inodes SET nlink=nlink+1,generation=generation+1,ctime=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(w.0).bind(source.id).fetch_one(&mut **tx).await?;
    touch_parent(tx, w, parent.id, 0).await?;
    guard.bump(tx).await?;
    Ok(mutated(&row))
}
async fn drop_link(
    tx: &mut Transaction<'_, Postgres>,
    w: WorkspaceId,
    row: &InodeRow,
) -> Result<InodeRow, sqlx::Error> {
    sqlx::query_as("UPDATE inodes SET nlink=$3,unlinked_at=CASE WHEN $3=0 THEN transaction_timestamp() ELSE NULL END,generation=generation+1,ctime=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(w.0).bind(row.id).bind(if row.kind=="directory" {0} else {row.nlink-1}).fetch_one(&mut **tx).await
}
pub async fn rmdir(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
) -> Result<Mutated, NamespaceError> {
    remove(tx, guard, caller, path, true).await
}
pub async fn unlink(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
) -> Result<Mutated, NamespaceError> {
    remove(tx, guard, caller, path, false).await
}
async fn remove(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: &WorkspacePath,
    directory: bool,
) -> Result<Mutated, NamespaceError> {
    let w = guard.workspace;
    let (parent, name) = resolve_parent(tx, w, path, caller).await?;
    let row = child(tx, w, parent.id, name.as_str())
        .await?
        .ok_or(NamespaceError::NotFound)?;
    let rows = lock_inodes(tx, w, &[parent.id, row.id]).await?;
    parent_access(locked(&rows, parent.id)?, caller)?;
    let row = locked(&rows, row.id)?;
    unchanged(tx, w, parent.id, name.as_str(), Some(row.id)).await?;
    if directory && row.kind != "directory" {
        return Err(NamespaceError::NotDirectory);
    }
    if !directory && row.kind == "directory" {
        return Err(NamespaceError::IsDirectory);
    }
    if directory && !empty(tx, w, row.id).await? {
        return Err(NamespaceError::DirectoryNotEmpty);
    }
    delete_entry(tx, w, parent.id, name.as_str()).await?;
    let row = drop_link(tx, w, row).await?;
    touch_parent(tx, w, parent.id, if directory { -1 } else { 0 }).await?;
    crate::quota::release_if_due(tx, w, &row).await?;
    guard.bump(tx).await?;
    Ok(mutated(&row))
}
pub async fn set_attributes(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    path: Option<&WorkspacePath>,
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
) -> Result<Mutated, NamespaceError> {
    if mode.is_none() && uid.is_none() && gid.is_none() {
        return Err(NamespaceError::InvalidInput);
    }
    if let Some(mode) = mode {
        validate_mode(mode)?;
    }
    let w = guard.workspace;
    let row = resolve(tx, w, path, caller).await?;
    let rows = lock_inodes(tx, w, &[row.id]).await?;
    let row = locked(&rows, row.id)?;
    if mode.is_some() && row.kind == "symlink" {
        return Err(NamespaceError::InvalidInput);
    }
    if !caller.admin
        && ((mode.is_some() && row.uid != i64::from(caller.identity.uid))
            || uid.is_some()
            || gid.is_some())
    {
        return Err(NamespaceError::Forbidden);
    }
    let row: InodeRow=sqlx::query_as("UPDATE inodes SET mode=coalesce($3,mode),uid=coalesce($4,uid),gid=coalesce($5,gid),ctime=transaction_timestamp(),generation=generation+1 WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(w.0).bind(row.id).bind(mode.map(|v|v as i32)).bind(uid.map(i64::from)).bind(gid.map(i64::from)).fetch_one(&mut **tx).await?;
    Ok(mutated(&row))
}
pub async fn rename(
    tx: &mut Transaction<'_, Postgres>,
    guard: &NamespaceGuard,
    caller: &Caller,
    from: &WorkspacePath,
    to: &WorkspacePath,
) -> Result<Mutated, NamespaceError> {
    let w = guard.workspace;
    let (pf, nf) = resolve_parent(tx, w, from, caller).await?;
    let (pt, nt) = resolve_parent(tx, w, to, caller).await?;
    let src = child(tx, w, pf.id, nf.as_str())
        .await?
        .ok_or(NamespaceError::NotFound)?;
    let dst = child(tx, w, pt.id, nt.as_str()).await?;
    if from == to || dst.as_ref().is_some_and(|d| d.id == src.id) {
        return Ok(mutated(&src));
    }
    let is_dir = src.kind == "directory";
    if is_dir && pf.id != pt.id {
        access(
            &src,
            caller,
            Access {
                write: true,
                ..Access::default()
            },
        )?;
        if ancestors(tx, w, pt.id).await?.contains(&src.id) {
            return Err(NamespaceError::InvalidInput);
        }
    }
    if let Some(dst) = &dst {
        if is_dir && dst.kind != "directory" {
            return Err(NamespaceError::NotDirectory);
        }
        if !is_dir && dst.kind == "directory" {
            return Err(NamespaceError::IsDirectory);
        }
        if dst.kind == "directory" && !empty(tx, w, dst.id).await? {
            return Err(NamespaceError::DirectoryNotEmpty);
        }
    }
    let mut ids = vec![src.id, pf.id, pt.id];
    if let Some(dst) = &dst {
        ids.push(dst.id);
    }
    let rows = lock_inodes(tx, w, &ids).await?;
    parent_access(locked(&rows, pf.id)?, caller)?;
    parent_access(locked(&rows, pt.id)?, caller)?;
    let src = locked(&rows, src.id)?;
    if is_dir && pf.id != pt.id {
        access(
            src,
            caller,
            Access {
                write: true,
                ..Access::default()
            },
        )?;
    }
    unchanged(tx, w, pf.id, nf.as_str(), Some(src.id)).await?;
    unchanged(tx, w, pt.id, nt.as_str(), dst.as_ref().map(|d| d.id)).await?;
    // Defer quota release until after all inode and entry updates.
    let replaced = if let Some(dst) = &dst {
        let dst = locked(&rows, dst.id)?;
        delete_entry(tx, w, pt.id, nt.as_str()).await?;
        Some(drop_link(tx, w, dst).await?)
    } else {
        None
    };
    sqlx::query("UPDATE directory_entries SET parent_inode_id=$4,name=$5 WHERE workspace_id=$1 AND parent_inode_id=$2 AND name=$3").bind(w.0).bind(pf.id).bind(nf.as_str()).bind(pt.id).bind(nt.as_str()).execute(&mut **tx).await?;
    let replacement_delta = if dst.as_ref().is_some_and(|d| d.kind == "directory") {
        -1
    } else {
        0
    };
    if pf.id == pt.id {
        touch_parent(tx, w, pf.id, replacement_delta).await?;
    } else {
        touch_parent(tx, w, pf.id, if is_dir { -1 } else { 0 }).await?;
        touch_parent(tx, w, pt.id, i32::from(is_dir) + replacement_delta).await?;
    }
    let row: InodeRow=sqlx::query_as("UPDATE inodes SET ctime=transaction_timestamp(),generation=generation+1 WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(w.0).bind(src.id).fetch_one(&mut **tx).await?;
    if let Some(replaced) = replaced {
        crate::quota::release_if_due(tx, w, &replaced).await?;
    }
    guard.bump(tx).await?;
    Ok(mutated(&row))
}
