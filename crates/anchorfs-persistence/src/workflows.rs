//! Independently designed lease state transitions. Callers hold the workspace
//! FOR UPDATE before credentials, sessions, and receipts, through commit.
use crate::authority::{
    self, ActorCredential, ActorSession, AuthorityError, CredentialKind, WorkflowProof,
    WorkspaceLock,
};
use anchorfs_core::{Secret, constant_time_eq, decode_hex_32, random_token, sha256};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    Free,
    Live,
    Dead,
}
#[derive(Debug)]
pub struct LeaseGrant {
    pub token: Secret<String>,
    pub mutation_epoch: i64,
    pub expires_at: OffsetDateTime,
}
#[derive(Debug, sqlx::FromRow)]
pub struct Renewed {
    pub mutation_epoch: i64,
    pub expires_at: OffsetDateTime,
}
#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error("lease_active")]
    LeaseActive,
    #[error("lease_expired")]
    LeaseExpiredBlocksAcquire,
    #[error("lease_expired")]
    LeaseExpiredFencesProof,
    #[error("lease_absent")]
    LeaseAbsent,
    #[error("session_bound")]
    SessionBound,
    #[error("workflow_proof_mismatch")]
    ProofMismatch,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
async fn state(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
) -> Result<LeaseState, WorkflowError> {
    let now = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    Ok(lock.lease_state(now))
}
fn eligible(actor: &ActorCredential, session: &ActorSession) -> Result<(), WorkflowError> {
    if actor.kind != CredentialKind::Host || !authority::role_permits_mutation(actor) {
        return Err(AuthorityError::Forbidden.into());
    }
    if session.bound_mutation_epoch.is_some() {
        return Err(WorkflowError::SessionBound);
    }
    Ok(())
}
fn matches(lock: &WorkspaceLock, token: &[u8; 32]) -> bool {
    lock.workflow_token_hash
        .as_ref()
        .is_some_and(|hash| constant_time_eq(&sha256(token).0, hash))
}
async fn owner_proof(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    proof: &WorkflowProof,
) -> Result<(), WorkflowError> {
    let state = state(tx, lock).await?;
    if state == LeaseState::Free {
        return Err(WorkflowError::LeaseAbsent);
    }
    if lock.workflow_owner_credential_id != Some(actor.id.0) {
        return Err(AuthorityError::Forbidden.into());
    }
    if state == LeaseState::Dead {
        return Err(WorkflowError::LeaseExpiredFencesProof);
    }
    if proof.epoch != lock.mutation_epoch || !matches(lock, &proof.token) {
        return Err(WorkflowError::ProofMismatch);
    }
    Ok(())
}
async fn grant(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    ttl: Duration,
) -> Result<LeaseGrant, WorkflowError> {
    let token = random_token();
    let hash = sha256(decode_hex_32(token.expose()).expect("generated hex")).0;
    let row: Renewed = sqlx::query_as("UPDATE workspaces SET workflow_owner_credential_id=$3, workflow_token_hash=$4, workflow_expires_at=clock_timestamp()+$5*interval '1 second', mutation_epoch=mutation_epoch+1, updated_at=transaction_timestamp() WHERE id=$1 AND mutation_epoch=$2 RETURNING mutation_epoch,workflow_expires_at AS expires_at")
        .bind(lock.id).bind(lock.mutation_epoch).bind(actor.id.0).bind(hash.as_slice()).bind(ttl.as_secs_f64()).fetch_optional(&mut **tx).await?.ok_or(AuthorityError::Fenced)?;
    Ok(LeaseGrant {
        token,
        mutation_epoch: row.mutation_epoch,
        expires_at: row.expires_at,
    })
}
pub async fn acquire(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    session: &ActorSession,
    ttl: Duration,
) -> Result<LeaseGrant, WorkflowError> {
    eligible(actor, session)?;
    match state(tx, lock).await? {
        LeaseState::Live => return Err(WorkflowError::LeaseActive),
        LeaseState::Dead => return Err(WorkflowError::LeaseExpiredBlocksAcquire),
        LeaseState::Free => (),
    }
    grant(tx, lock, actor, ttl).await
}
pub async fn takeover(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    session: &ActorSession,
    ttl: Duration,
) -> Result<LeaseGrant, WorkflowError> {
    eligible(actor, session)?;
    match state(tx, lock).await? {
        LeaseState::Live => return Err(WorkflowError::LeaseActive),
        LeaseState::Free => return Err(WorkflowError::LeaseAbsent),
        LeaseState::Dead => (),
    }
    grant(tx, lock, actor, ttl).await
}
pub async fn renew(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    session: &ActorSession,
    proof: &WorkflowProof,
    ttl: Duration,
) -> Result<Renewed, WorkflowError> {
    eligible(actor, session)?;
    owner_proof(tx, lock, actor, proof).await?;
    Ok(sqlx::query_as("UPDATE workspaces SET workflow_expires_at=clock_timestamp()+$3*interval '1 second',updated_at=transaction_timestamp() WHERE id=$1 AND mutation_epoch=$2 RETURNING mutation_epoch,workflow_expires_at AS expires_at")
        .bind(lock.id).bind(lock.mutation_epoch).bind(ttl.as_secs_f64()).fetch_optional(&mut **tx).await?.ok_or(AuthorityError::Fenced)?)
}
pub async fn release(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    session: &ActorSession,
    proof: &WorkflowProof,
) -> Result<i64, WorkflowError> {
    eligible(actor, session)?;
    owner_proof(tx, lock, actor, proof).await?;
    Ok(sqlx::query_scalar("UPDATE workspaces SET workflow_owner_credential_id=NULL,workflow_token_hash=NULL,workflow_expires_at=NULL,mutation_epoch=mutation_epoch+1,updated_at=transaction_timestamp() WHERE id=$1 AND mutation_epoch=$2 RETURNING mutation_epoch")
        .bind(lock.id).bind(lock.mutation_epoch).fetch_optional(&mut **tx).await?.ok_or(AuthorityError::Fenced)?)
}
/// A shared workspace lock suffices for binding; existing session checks run first.
pub async fn bind_epoch(
    tx: &mut Transaction<'_, Postgres>,
    lock: &WorkspaceLock,
    actor: &ActorCredential,
    token: &str,
) -> Result<i64, WorkflowError> {
    match state(tx, lock).await? {
        LeaseState::Free => return Err(WorkflowError::LeaseAbsent),
        LeaseState::Dead => return Err(WorkflowError::LeaseExpiredFencesProof),
        LeaseState::Live => (),
    }
    if actor.kind != CredentialKind::Host || lock.workflow_owner_credential_id != Some(actor.id.0) {
        return Err(AuthorityError::Forbidden.into());
    }
    let raw = decode_hex_32(token).ok_or(AuthorityError::InvalidInput)?;
    if !matches(lock, &raw) {
        return Err(WorkflowError::ProofMismatch);
    }
    Ok(lock.mutation_epoch)
}
