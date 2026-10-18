//! Session minting limits serialize per target credential under a transaction advisory lock.
use crate::{
    authority::{ActorCredential, ActorSession, AuthorityError, CredentialKind, SessionRow},
    rows::CredentialRow,
};
use anchorfs_core::{CredentialId, SessionId, WorkspaceId};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug)]
pub struct SessionView {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub credential_id: CredentialId,
    pub credential_epoch: i64,
    pub session_epoch: i64,
    pub bound_mutation_epoch: Option<i64>,
    pub expires_at: OffsetDateTime,
}
impl From<SessionRow> for SessionView {
    fn from(row: SessionRow) -> Self {
        Self {
            id: SessionId(row.id),
            workspace_id: WorkspaceId(row.workspace_id),
            credential_id: CredentialId(row.credential_id),
            credential_epoch: row.credential_epoch,
            session_epoch: row.session_epoch,
            bound_mutation_epoch: row.bound_mutation_epoch,
            expires_at: row.expires_at,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error("session limit reached")]
    Limit,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
pub async fn count_active(
    tx: &mut Transaction<'_, Postgres>,
    credential: CredentialId,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp()")
        .bind(credential.0).fetch_one(&mut **tx).await
}
pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    workspace: WorkspaceId,
    target: CredentialId,
    ttl: Duration,
    limit: u32,
) -> Result<SessionView, SessionError> {
    if actor.kind == CredentialKind::Worker {
        return Err(AuthorityError::Forbidden.into());
    }
    if !crate::authority::same_workspace(actor, workspace) {
        return Err(AuthorityError::NotFound.into());
    }
    let row = sqlx::query_as::<_, CredentialRow>(
        "SELECT * FROM credentials WHERE workspace_id=$1 AND id=$2 FOR SHARE",
    )
    .bind(workspace.0)
    .bind(target.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AuthorityError::NotFound)?;
    let expired: bool =
        sqlx::query_scalar("SELECT $1::timestamptz IS NOT NULL AND $1<=clock_timestamp()")
            .bind(row.expires_at)
            .fetch_one(&mut **tx)
            .await?;
    if row.revoked_at.is_some() || expired || row.kind == "administrator" {
        return Err(AuthorityError::NotFound.into());
    }
    lock_admission(tx, target).await?;
    if count_active(tx, target).await? >= i64::from(limit) {
        return Err(SessionError::Limit);
    }
    let id = SessionId::new();
    let credential_expiry = row.expires_at;
    let row=sqlx::query_as::<_, SessionRow>("INSERT INTO client_sessions (id,workspace_id,credential_id,credential_epoch,session_epoch,bound_mutation_epoch,expires_at) VALUES ($1,$2,$3,$4,1,NULL,clock_timestamp()+$5*interval '1 second') RETURNING *")
        .bind(id.0).bind(workspace.0).bind(target.0).bind(row.epoch).bind(ttl.as_secs_f64()).fetch_one(&mut **tx).await?;
    crate::authority::check_publication_time(tx, actor, None).await?;
    let target_expired: bool =
        sqlx::query_scalar("SELECT $1::timestamptz IS NOT NULL AND $1<=clock_timestamp()")
            .bind(credential_expiry)
            .fetch_one(&mut **tx)
            .await?;
    if target_expired {
        return Err(AuthorityError::NotFound.into());
    }
    Ok(row.into())
}
async fn target(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    target: SessionId,
) -> Result<SessionRow, SessionError> {
    sqlx::query_as("SELECT * FROM client_sessions WHERE workspace_id=$1 AND id=$2 FOR UPDATE")
        .bind(workspace.0)
        .bind(target.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(AuthorityError::NotFound.into())
}
fn authorize(
    actor: &ActorCredential,
    actor_session: &ActorSession,
    target: SessionId,
    rebind: bool,
) -> Result<(), AuthorityError> {
    if actor.kind == CredentialKind::Host
        || (!rebind && actor.kind == CredentialKind::Worker && actor_session.id == target)
    {
        Ok(())
    } else {
        Err(AuthorityError::Forbidden)
    }
}
async fn expired(
    tx: &mut Transaction<'_, Postgres>,
    row: &SessionRow,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT $1::timestamptz<=clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await
}
pub async fn renew(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    actor_session: &ActorSession,
    workspace: WorkspaceId,
    id: SessionId,
    ttl: Duration,
) -> Result<SessionView, SessionError> {
    let row = target(tx, workspace, id).await?;
    authorize(actor, actor_session, id, false)?;
    // Hold admission through COMMIT: minters must not count an old MVCC expiry
    // while this transaction has extended the session lifetime.
    lock_admission(tx, CredentialId(row.credential_id)).await?;
    if row.revoked_at.is_some() || expired(tx, &row).await? {
        return Err(AuthorityError::Fenced.into());
    }
    let row=sqlx::query_as::<_,SessionRow>("UPDATE client_sessions SET expires_at=clock_timestamp()+$3*interval '1 second' WHERE workspace_id=$1 AND id=$2 RETURNING *")
        .bind(workspace.0).bind(id.0).bind(ttl.as_secs_f64()).fetch_one(&mut **tx).await?;
    crate::authority::check_publication_time(tx, actor, Some(actor_session)).await?;
    Ok(row.into())
}
pub async fn rebind(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    actor_session: &ActorSession,
    workspace: WorkspaceId,
    id: SessionId,
    ttl: Duration,
    limit: u32,
) -> Result<SessionView, SessionError> {
    authorize(actor, actor_session, id, true)?;
    let row = target(tx, workspace, id).await?;
    if row.revoked_at.is_some() {
        return Err(AuthorityError::Fenced.into());
    }
    let credential = CredentialId(row.credential_id);
    lock_admission(tx, credential).await?;
    if expired(tx, &row).await? && count_active(tx, credential).await? >= i64::from(limit) {
        return Err(SessionError::Limit);
    }
    let row=sqlx::query_as::<_,SessionRow>("UPDATE client_sessions SET session_epoch=session_epoch+1,bound_mutation_epoch=NULL,expires_at=clock_timestamp()+$3*interval '1 second' WHERE workspace_id=$1 AND id=$2 RETURNING *")
        .bind(workspace.0).bind(id.0).bind(ttl.as_secs_f64()).fetch_one(&mut **tx).await?;
    crate::authority::check_publication_time(tx, actor, Some(actor_session)).await?;
    Ok(row.into())
}
pub async fn revoke(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    actor_session: &ActorSession,
    workspace: WorkspaceId,
    id: SessionId,
) -> Result<SessionView, SessionError> {
    let row = target(tx, workspace, id).await?;
    authorize(actor, actor_session, id, false)?;
    if row.revoked_at.is_some() {
        return Err(AuthorityError::NotFound.into());
    }
    let row=sqlx::query_as::<_,SessionRow>("UPDATE client_sessions SET revoked_at=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *")
        .bind(workspace.0).bind(id.0).fetch_one(&mut **tx).await?;
    crate::authority::check_publication_time(tx, actor, Some(actor_session)).await?;
    Ok(row.into())
}

/// Serialize insertion and every expiry extension until COMMIT, including the
/// MVCC interval where other transactions still see an old committed expiry.
async fn lock_admission(
    tx: &mut Transaction<'_, Postgres>,
    credential: CredentialId,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,7001))")
        .bind(credential.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
