//! Upload admission and fencing primitives. Transactions that also take quota
//! or operation locks must take the operation lock first; quota locks are last.
use crate::authority::ActorCredential;
use anchorfs_core::{OperationId, ReservationId, WorkspaceId};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReservationRow {
    pub workspace_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub operation_id: uuid::Uuid,
    pub credential_id: uuid::Uuid,
    pub reserved_bytes: i64,
    pub state: String,
    pub heartbeat_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    WorkspaceStaging,
    WorkspacePhysical,
    DeploymentStaging,
    DeploymentPhysical,
}
#[derive(Debug, thiserror::Error)]
pub enum ReservationError {
    #[error("upload quota exceeded: {0:?}")]
    Exceeded(QuotaKind),
    #[error("principal reservation bound exceeded")]
    PrincipalBound,
    #[error("reservation expired")]
    Expired,
    #[error("reservation is not active")]
    NotActive,
    #[error("reservation not found")]
    NotFound,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    operation: OperationId,
    actor: &ActorCredential,
    bytes: i64,
    ttl: Duration,
) -> Result<ReservationRow, ReservationError> {
    if bytes <= 0 || bytes > actor.max_reservation_bytes {
        return Err(ReservationError::PrincipalBound);
    }
    Ok(sqlx::query_as("INSERT INTO upload_reservations(workspace_id,id,operation_id,credential_id,reserved_bytes,state,heartbeat_at,expires_at) VALUES($1,$2,$3,$4,$5,'active',clock_timestamp(),clock_timestamp()+$6 * interval '1 second') RETURNING *")
        .bind(workspace.0).bind(ReservationId::new().0).bind(operation.0).bind(actor.id.0)
        .bind(bytes).bind(ttl.as_secs_f64()).fetch_one(&mut **tx).await?)
}

pub async fn charge(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    row: &ReservationRow,
) -> Result<(), ReservationError> {
    crate::quota::reserve_staging(tx, workspace, row.reserved_bytes).await
}

async fn locked(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    id: ReservationId,
    operation: OperationId,
    share: bool,
    check_expiry: bool,
) -> Result<ReservationRow, ReservationError> {
    let query = if share {
        "SELECT * FROM upload_reservations WHERE workspace_id=$1 AND id=$2 FOR SHARE"
    } else {
        "SELECT * FROM upload_reservations WHERE workspace_id=$1 AND id=$2 FOR UPDATE"
    };
    let row: ReservationRow = sqlx::query_as(query)
        .bind(workspace.0)
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ReservationError::NotFound)?;
    if row.state != "active" || row.operation_id != operation.0 {
        return Err(ReservationError::NotActive);
    }
    if check_expiry {
        let expired: bool = sqlx::query_scalar("SELECT $1::timestamptz <= clock_timestamp()")
            .bind(row.expires_at)
            .fetch_one(&mut **tx)
            .await?;
        if expired {
            return Err(ReservationError::Expired);
        }
    }
    Ok(row)
}
pub async fn lock_active(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    id: ReservationId,
    operation: OperationId,
) -> Result<ReservationRow, ReservationError> {
    locked(tx, workspace, id, operation, false, true).await
}
pub async fn lock_for_finalize(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    id: ReservationId,
    operation: OperationId,
) -> Result<ReservationRow, ReservationError> {
    locked(tx, workspace, id, operation, false, false).await
}
pub async fn lock_active_share(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    id: ReservationId,
    operation: OperationId,
) -> Result<(), ReservationError> {
    locked(tx, workspace, id, operation, true, true).await?;
    Ok(())
}
pub async fn consume(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    row: &ReservationRow,
) -> Result<(), sqlx::Error> {
    transition(tx, workspace, row, "consumed").await
}
pub async fn fail(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    row: &ReservationRow,
    state: &'static str,
) -> Result<(), sqlx::Error> {
    if !matches!(state, "failed" | "expired") {
        return Err(sqlx::Error::Protocol(
            "invalid reservation failure state".into(),
        ));
    }
    transition(tx, workspace, row, state).await
}
async fn transition(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    row: &ReservationRow,
    state: &str,
) -> Result<(), sqlx::Error> {
    let affected = sqlx::query("UPDATE upload_reservations SET state=$3 WHERE workspace_id=$1 AND id=$2 AND state='active' AND operation_id=$4")
        .bind(workspace.0).bind(row.id).bind(state).bind(row.operation_id).execute(&mut **tx).await?.rows_affected();
    if affected != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

/// Best-effort bounded lazy expiry; every fence gets its own committing transaction.
pub async fn fence_overdue(
    pool: &sqlx::PgPool,
    workspace: Option<WorkspaceId>,
    limit: u32,
) -> Result<u32, sqlx::Error> {
    let overdue: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as("SELECT DISTINCT workspace_id,operation_id FROM upload_reservations WHERE state='active' AND expires_at<=clock_timestamp() AND ($1::uuid IS NULL OR workspace_id=$1) LIMIT $2")
        .bind(workspace.map(|w|w.0)).bind(i64::from(limit)).fetch_all(pool).await?;
    let mut count = 0;
    for (workspace, operation) in overdue {
        let mut tx = pool.begin().await?;
        if matches!(
            crate::receipts::fence_expired_pending(
                &mut tx,
                WorkspaceId(workspace),
                OperationId(operation)
            )
            .await?,
            crate::receipts::FenceOutcome::Fenced
        ) {
            count += 1;
        }
        tx.commit().await?;
    }
    Ok(count)
}
