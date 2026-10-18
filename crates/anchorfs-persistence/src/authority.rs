// SQL audit: interpolated identifiers/clauses come only from private enum/static choices.
// All request data uses bind parameters.
//! Credential and session checks performed again under transaction row locks.
use crate::rows::CredentialRow;
use anchorfs_core::{
    CredentialId, CredentialProof, SessionId, WorkspaceId, constant_time_eq, secret_hash,
};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy)]
pub enum LockMode {
    Share,
    Update,
}
impl LockMode {
    pub(crate) fn clause(self) -> &'static str {
        match self {
            Self::Share => "FOR SHARE",
            Self::Update => "FOR UPDATE",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Administrator,
    Host,
    Worker,
}
impl TryFrom<&str> for CredentialKind {
    type Error = AuthorityError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "administrator" => Ok(Self::Administrator),
            "host" => Ok(Self::Host),
            "worker" => Ok(Self::Worker),
            _ => Err(AuthorityError::Forbidden),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CredentialRole {
    Read,
    Write,
    Admin,
}
impl TryFrom<&str> for CredentialRole {
    type Error = AuthorityError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "admin" => Ok(Self::Admin),
            _ => Err(AuthorityError::Forbidden),
        }
    }
}
#[derive(Debug, Clone)]
pub struct ActorCredential {
    pub id: CredentialId,
    pub workspace_id: Option<WorkspaceId>,
    pub kind: CredentialKind,
    pub role: CredentialRole,
    pub uid: u32,
    pub gid: u32,
    pub epoch: i64,
    pub issued_by: Option<CredentialId>,
    pub max_reservation_bytes: i64,
    pub expires_at: Option<OffsetDateTime>,
}
#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("workspace_busy")]
    WorkspaceBusy,
    #[error("fenced")]
    Fenced,
    #[error("forbidden")]
    Forbidden,
    #[error("not_found")]
    NotFound,
    #[error("invalid_input")]
    InvalidInput,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
impl TryFrom<CredentialRow> for ActorCredential {
    type Error = AuthorityError;
    fn try_from(row: CredentialRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: CredentialId(row.id),
            workspace_id: row.workspace_id.map(WorkspaceId),
            kind: row.kind.as_str().try_into()?,
            role: row.role.as_str().try_into()?,
            uid: row.uid as u32,
            gid: row.gid as u32,
            epoch: row.epoch,
            issued_by: row.issued_by_credential_id.map(CredentialId),
            max_reservation_bytes: row.max_reservation_bytes,
            expires_at: row.expires_at,
        })
    }
}
async fn credential(
    tx: &mut Transaction<'_, Postgres>,
    proof: &CredentialProof,
    lock: &str,
) -> Result<ActorCredential, AuthorityError> {
    // The expired flag is evaluated by the database clock, including after lock waits.
    let row = sqlx::query_as::<_, CredentialRow>(sqlx::AssertSqlSafe(format!(
        "SELECT c.* FROM credentials c WHERE id=$1 {lock}"
    )))
    .bind(proof.id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AuthorityError::Unauthenticated)?;
    let expired: bool =
        sqlx::query_scalar("SELECT $1::timestamptz IS NOT NULL AND $1 <= clock_timestamp()")
            .bind(row.expires_at)
            .fetch_one(&mut **tx)
            .await?;
    if !constant_time_eq(&secret_hash(proof.secret.expose()).0, &row.secret_hash)
        || row.revoked_at.is_some()
        || expired
    {
        return Err(AuthorityError::Unauthenticated);
    }
    row.try_into()
}
pub async fn load_actor(
    tx: &mut Transaction<'_, Postgres>,
    proof: &CredentialProof,
    mode: LockMode,
) -> Result<ActorCredential, AuthorityError> {
    credential(tx, proof, "").await?;
    credential(tx, proof, mode.clause()).await
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub credential_id: uuid::Uuid,
    pub session_epoch: i64,
    pub credential_epoch: i64,
    pub bound_mutation_epoch: Option<i64>,
    pub expires_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}
#[derive(Debug, Clone)]
pub struct ActorSession {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub credential_id: CredentialId,
    pub session_epoch: i64,
    pub credential_epoch: i64,
    pub bound_mutation_epoch: Option<i64>,
    pub expires_at: OffsetDateTime,
}
pub async fn load_actor_session(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    workspace: WorkspaceId,
    session: SessionId,
    header_epoch: i64,
    mode: LockMode,
) -> Result<ActorSession, AuthorityError> {
    let row = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT * FROM client_sessions WHERE workspace_id=$1 AND id=$2 {}",
        mode.clause()
    )))
    .bind(workspace.0)
    .bind(session.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AuthorityError::Unauthenticated)?;
    if row.credential_id != actor.id.0 {
        return Err(AuthorityError::Unauthenticated);
    }
    let expired: bool = sqlx::query_scalar("SELECT $1::timestamptz <= clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await?;
    if row.revoked_at.is_some()
        || expired
        || row.session_epoch != header_epoch
        || row.credential_epoch != actor.epoch
    {
        return Err(AuthorityError::Fenced);
    }
    Ok(ActorSession {
        id: session,
        workspace_id: workspace,
        credential_id: actor.id,
        session_epoch: row.session_epoch,
        credential_epoch: row.credential_epoch,
        bound_mutation_epoch: row.bound_mutation_epoch,
        expires_at: row.expires_at,
    })
}
#[derive(Debug, sqlx::FromRow)]
pub struct WorkspaceLock {
    pub workflow_owner_credential_id: Option<uuid::Uuid>,
    pub workflow_token_hash: Option<Vec<u8>>,
    pub workflow_expires_at: Option<OffsetDateTime>,
    pub id: uuid::Uuid,
    pub authorization_epoch: i64,
    pub mutation_epoch: i64,
    pub state: String,
}
pub async fn lock_workspace(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
) -> Result<Option<WorkspaceLock>, sqlx::Error> {
    sqlx::query_as("SELECT id, authorization_epoch, mutation_epoch, state, workflow_owner_credential_id, workflow_token_hash, workflow_expires_at FROM workspaces WHERE id=$1 FOR SHARE")
        .bind(workspace.0).fetch_optional(&mut **tx).await
}
pub fn same_workspace(actor: &ActorCredential, workspace: WorkspaceId) -> bool {
    actor.kind == CredentialKind::Administrator || actor.workspace_id == Some(workspace)
}

/// Row locks freeze authority fields, not database time. Check the admitted
/// lifetimes again after waits, using the original session even for self-renew.
/// Intentionally do not reload post-mutation epochs/revocation: self-fencing
/// operations must be able to publish their receipt.
pub async fn check_publication_time(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    session: Option<&ActorSession>,
) -> Result<(), AuthorityError> {
    let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    if actor.expires_at.is_some_and(|expiry| expiry <= now) {
        return Err(AuthorityError::Unauthenticated);
    }
    if session.is_some_and(|session| session.expires_at <= now) {
        return Err(AuthorityError::Fenced);
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct WorkflowProof {
    pub token: [u8; 32],
    pub epoch: i64,
}
#[derive(Debug, Clone, Copy)]
pub enum LeaseMode {
    Mutation,
    Read,
}
pub async fn check_workflow(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    session: &ActorSession,
    proof: Option<&WorkflowProof>,
    mode: LeaseMode,
) -> Result<(), AuthorityError> {
    let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    if session
        .bound_mutation_epoch
        .is_some_and(|e| e != lock.mutation_epoch)
    {
        return Err(AuthorityError::Fenced);
    }
    if matches!(mode, LeaseMode::Read) {
        return Ok(());
    }
    let active = lock.workflow_expires_at.is_some_and(|t| t > now);
    let valid = proof.is_some_and(|p| {
        active
            && p.epoch == lock.mutation_epoch
            && lock
                .workflow_token_hash
                .as_ref()
                .is_some_and(|hash| constant_time_eq(&anchorfs_core::sha256(p.token).0, hash))
            && (lock.workflow_owner_credential_id == Some(actor.id.0)
                || session.bound_mutation_epoch == Some(p.epoch))
    });
    if session.bound_mutation_epoch.is_some() && !valid {
        return Err(AuthorityError::Fenced);
    }
    if active && !valid {
        return Err(AuthorityError::WorkspaceBusy);
    }
    if !active && proof.is_some() {
        return Err(AuthorityError::Fenced);
    }
    Ok(())
}
pub fn role_permits_mutation(actor: &ActorCredential) -> bool {
    actor.role >= CredentialRole::Write
}
pub fn is_workspace_admin(actor: &ActorCredential) -> bool {
    actor.kind == CredentialKind::Host && actor.role == CredentialRole::Admin
}
pub fn identity(actor: &ActorCredential) -> anchorfs_core::Identity {
    anchorfs_core::Identity {
        uid: actor.uid,
        gid: actor.gid,
    }
}
