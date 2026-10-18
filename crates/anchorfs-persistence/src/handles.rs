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

use crate::{
    authority::{ActorSession, LockMode},
    namespace::{self, Caller, InodeRow, NamespaceError, NamespaceGuard},
};
use anchorfs_core::{Access, HandleId, SessionId, WorkspacePath};
use std::time::Duration;
use time::OffsetDateTime;
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HandleRow {
    pub workspace_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub inode_id: i64,
    pub client_session_id: uuid::Uuid,
    pub credential_id: uuid::Uuid,
    pub access_read: bool,
    pub access_write: bool,
    pub opened_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub closed_at: Option<OffsetDateTime>,
}
#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error("stale_handle")]
    Stale,
    #[error("handle limit")]
    Limit,
    #[error("not_found")]
    NotFound,
    #[error(transparent)]
    Namespace(#[from] NamespaceError),
    #[error(transparent)]
    Quota(#[from] crate::quota::QuotaError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
pub struct OpenRequest {
    pub path: WorkspacePath,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub mode: Option<u32>,
}
#[derive(Clone, Copy)]
pub struct HandlePolicy {
    pub ttl: Duration,
    pub limit: u32,
}
pub struct Opened {
    pub handle: HandleRow,
    pub inode: InodeRow,
    pub created: bool,
}
pub fn validate_request(req: &OpenRequest) -> Result<(), NamespaceError> {
    if (!req.read && !req.write)
        || (req.exclusive && !req.create)
        || (req.truncate && !req.write)
        || (req.create && req.mode.is_none())
    {
        return Err(NamespaceError::InvalidInput);
    }
    if req.create {
        namespace::validate_mode(req.mode.expect("checked mode"))?;
    }
    Ok(())
}
pub async fn lock_admission(
    tx: &mut Transaction<'_, Postgres>,
    session: SessionId,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,7002))")
        .bind(session.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn count_open(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: SessionId,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM handles WHERE workspace_id=$1 AND client_session_id=$2 AND closed_at IS NULL AND expires_at>clock_timestamp()").bind(workspace.0).bind(session.0).fetch_one(&mut **tx).await
}
pub async fn open_file(
    tx: &mut Transaction<'_, Postgres>,
    guard: Option<&NamespaceGuard>,
    caller: &Caller,
    session: &ActorSession,
    req: OpenRequest,
    policy: HandlePolicy,
) -> Result<Opened, HandleError> {
    validate_request(&req)?;
    let w = session.workspace_id;
    let existing = if req.create {
        if guard.is_none_or(|g| g.workspace() != w) {
            return Err(NamespaceError::InvalidInput.into());
        }
        let (parent, name) = namespace::resolve_parent(tx, w, &req.path, caller).await?;
        namespace::child(tx, w, parent.id, name.as_str()).await?
    } else {
        Some(namespace::resolve(tx, w, Some(&req.path), caller).await?)
    };
    let (mut inode, created) = match existing {
        Some(row) => {
            if req.exclusive {
                return Err(NamespaceError::Exists.into());
            }
            let mut rows = namespace::lock_inodes(tx, w, &[row.id]).await?;
            let row = rows
                .pop()
                .filter(|r| r.nlink > 0)
                .ok_or(HandleError::NotFound)?;
            if row.kind == "directory" {
                return Err(NamespaceError::IsDirectory.into());
            }
            if row.kind != "file" {
                return Err(NamespaceError::InvalidInput.into());
            }
            namespace::access(
                &row,
                caller,
                Access {
                    read: req.read,
                    write: req.write,
                    exec: false,
                },
            )?;
            (row, false)
        }
        None => {
            let row = namespace::create_file(
                tx,
                guard.expect("create guard"),
                caller,
                &req.path,
                req.mode.expect("validated mode"),
            )
            .await?;
            (row, true)
        }
    };
    if req.truncate {
        if inode.current_manifest_id.is_some() {
            return Err(NamespaceError::NotImplemented.into());
        }
        inode=sqlx::query_as("UPDATE inodes SET size=0,generation=generation+1,mtime=transaction_timestamp(),ctime=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(w.0).bind(inode.id).fetch_one(&mut **tx).await?;
    }
    lock_admission(tx, session.id).await?;
    if count_open(tx, w, session.id).await? >= i64::from(policy.limit) {
        return Err(HandleError::Limit);
    }
    let handle=sqlx::query_as("INSERT INTO handles(workspace_id,id,inode_id,client_session_id,credential_id,access_read,access_write,opened_at,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,clock_timestamp(),clock_timestamp()+$8::bigint*interval '1 millisecond') RETURNING *").bind(w.0).bind(HandleId::new().0).bind(inode.id).bind(session.id.0).bind(session.credential_id.0).bind(req.read).bind(req.write).bind(ttl_ms(policy.ttl)).fetch_one(&mut **tx).await?;
    Ok(Opened {
        handle,
        inode,
        created,
    })
}
fn ttl_ms(ttl: Duration) -> i64 {
    ttl.as_millis().min(i64::MAX as u128) as i64
}
/// SQL audit: lock clauses come solely from LockMode, never request text.
pub async fn load_for_actor(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: HandleId,
    lock: Option<LockMode>,
) -> Result<HandleRow, HandleError> {
    let row:HandleRow=sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT * FROM handles WHERE workspace_id=$1 AND id=$2 AND client_session_id=$3 AND credential_id=$4 {}",lock.map_or("",LockMode::clause)))).bind(workspace.0).bind(id.0).bind(session.id.0).bind(session.credential_id.0).fetch_optional(&mut **tx).await?.ok_or(HandleError::NotFound)?;
    let expired: bool = sqlx::query_scalar("SELECT $1::timestamptz<=clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await?;
    if expired || row.closed_at.is_some() {
        return Err(HandleError::Stale);
    }
    Ok(row)
}
async fn lock_handle_inode(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: HandleId,
) -> Result<InodeRow, HandleError> {
    let inode:i64=sqlx::query_scalar("SELECT inode_id FROM handles WHERE workspace_id=$1 AND id=$2 AND client_session_id=$3 AND credential_id=$4").bind(workspace.0).bind(id.0).bind(session.id.0).bind(session.credential_id.0).fetch_optional(&mut **tx).await?.ok_or(HandleError::NotFound)?;
    namespace::lock_inodes(tx, workspace, &[inode])
        .await?
        .pop()
        .ok_or(HandleError::NotFound)
}
pub async fn renew(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: HandleId,
    ttl: Duration,
) -> Result<(HandleRow, InodeRow), HandleError> {
    let inode = lock_handle_inode(tx, workspace, session, id).await?;
    let row = load_for_actor(tx, workspace, session, id, Some(LockMode::Update)).await?;
    if row.inode_id != inode.id {
        return Err(HandleError::NotFound);
    }
    let handle=sqlx::query_as("UPDATE handles SET expires_at=clock_timestamp()+$3::bigint*interval '1 millisecond' WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(workspace.0).bind(id.0).bind(ttl_ms(ttl)).fetch_one(&mut **tx).await?;
    lock_admission(tx, session.id).await?;
    Ok((handle, inode))
}
pub async fn close(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: HandleId,
) -> Result<(HandleRow, InodeRow), HandleError> {
    let inode = lock_handle_inode(tx, workspace, session, id).await?;
    let row = load_for_actor(tx, workspace, session, id, Some(LockMode::Update)).await?;
    if row.inode_id != inode.id {
        return Err(HandleError::NotFound);
    }
    let handle=sqlx::query_as("UPDATE handles SET closed_at=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *").bind(workspace.0).bind(id.0).fetch_one(&mut **tx).await?;
    crate::quota::release_if_due(tx, workspace, &inode).await?;
    Ok((handle, inode))
}
