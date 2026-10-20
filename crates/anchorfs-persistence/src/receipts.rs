// SQL audit: interpolated identifiers/clauses come only from private enum/static choices.
// All request data uses bind parameters.
//! Metadata receipts publish in one transaction; byte receipts retain a durable
//! owner across upload transactions. Every terminal transition is conditional.
use anchorfs_core::{
    CredentialId, OperationId, Secret, SessionId, WorkspaceId, random_token, sha256,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy)]
pub enum ReceiptScope {
    Admin {
        credential: CredentialId,
    },
    Workspace {
        workspace: WorkspaceId,
        credential: CredentialId,
        session: SessionId,
        session_epoch: i64,
        authorization_epoch: i64,
        mutation_epoch: i64,
    },
}
impl ReceiptScope {
    fn table(self) -> &'static str {
        match self {
            Self::Admin { .. } => "admin_operations",
            Self::Workspace { .. } => "operations",
        }
    }
    fn credential(self) -> CredentialId {
        match self {
            Self::Admin { credential } | Self::Workspace { credential, .. } => credential,
        }
    }
    fn workspace(self) -> Option<uuid::Uuid> {
        match self {
            Self::Admin { .. } => None,
            Self::Workspace { workspace, .. } => Some(workspace.0),
        }
    }
}
pub struct ReceiptRequest<'a> {
    pub key: &'a str,
    pub op_type: &'a str,
    pub digest: [u8; 32],
    pub credential_epoch: i64,
    pub retention: Duration,
    pub wait_bound: Duration,
}
#[derive(Debug)]
pub enum ReceiptEntry {
    Fresh {
        operation_id: OperationId,
        owner: Secret<String>,
    },
    Pending {
        operation_id: OperationId,
        reservation_overdue: bool,
    },
    Failed {
        operation_id: OperationId,
    },
    Replay {
        operation_id: OperationId,
        result: Value,
    },
    Mismatch {
        operation_id: OperationId,
    },
    Expired {
        operation_id: OperationId,
        tombstoned: Option<Tombstoned>,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("pending receipt owner")]
    PendingOwner,
    #[error("operation owner no longer holds publication authority")]
    NotOwner,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Stored {
    pub id: uuid::Uuid,
    pub credential_id: uuid::Uuid,
    pub client_session_id: Option<uuid::Uuid>,
    pub request_digest: Vec<u8>,
    pub state: String,
    pub result: Option<Value>,
    pub expires_at: OffsetDateTime,
    pub expiry_fences_session: bool,
}
#[derive(Debug, Clone)]
pub struct Tombstoned {
    pub operation_id: OperationId,
    pub prior_state: String,
    pub session: Option<SessionId>,
}
#[derive(Debug)]
pub struct ReceiptView {
    pub operation_id: OperationId,
    pub state: String,
    pub result: Option<Value>,
    pub tombstoned: Option<Tombstoned>,
}
fn columns(workspace: bool) -> &'static str {
    if workspace {
        "id,credential_id,client_session_id,request_digest,state,result,expires_at,expiry_fences_session"
    } else {
        "id,credential_id,NULL::uuid AS client_session_id,request_digest,state,result,expires_at,false AS expiry_fences_session"
    }
}
async fn is_expired(tx: &mut Transaction<'_, Postgres>, row: &Stored) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT $1::timestamptz <= clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await
}
fn followup(row: &Stored, prior_state: &str) -> Tombstoned {
    Tombstoned {
        operation_id: OperationId(row.id),
        prior_state: prior_state.into(),
        session: if row.expiry_fences_session {
            row.client_session_id.map(SessionId)
        } else {
            None
        },
    }
}
async fn tombstone(
    tx: &mut Transaction<'_, Postgres>,
    workspace: Option<WorkspaceId>,
    row: &mut Stored,
) -> Result<Option<Tombstoned>, sqlx::Error> {
    if row.state == "indeterminate" {
        return Ok(Some(followup(row, "indeterminate")));
    }
    if !is_expired(tx, row).await? {
        return Ok(None);
    }
    let prior_state = row.state.clone();
    if let Some(w) = workspace {
        if row.state == "pending" {
            match fence_expired_pending(tx, w, OperationId(row.id)).await? {
                FenceOutcome::Fenced => {
                    row.state = "failed".into();
                    row.result = Some(serde_json::json!({"code":"reservation_expired"}));
                }
                FenceOutcome::StillActive => {
                    return Err(sqlx::Error::Protocol(
                        "expired receipt has live reservation".into(),
                    ));
                }
                FenceOutcome::AlreadyResolved(resolved) => *row = resolved,
            }
        }
        let affected = sqlx::query("UPDATE operations SET state='indeterminate',result=NULL,expiry_fences_session=$4 WHERE workspace_id=$1 AND id=$2 AND state=$3")
            .bind(w.0).bind(row.id).bind(&row.state)
            .bind(matches!(prior_state.as_str(), "pending" | "committed"))
            .execute(&mut **tx).await?.rows_affected();
        if affected != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        row.expiry_fences_session = matches!(prior_state.as_str(), "pending" | "committed");
    } else {
        let affected = sqlx::query("UPDATE admin_operations SET state='indeterminate',result=NULL WHERE id=$1 AND state=$2")
            .bind(row.id).bind(&row.state).execute(&mut **tx).await?.rows_affected();
        if affected != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
    }
    row.state = "indeterminate".into();
    row.result = None;
    Ok(Some(followup(row, &prior_state)))
}
async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    key: &str,
    share: bool,
) -> Result<Option<Stored>, sqlx::Error> {
    let filter = if scope.workspace().is_some() {
        "workspace_id=$3"
    } else {
        "$3::uuid IS NULL"
    };
    let lock = if share { "FOR SHARE" } else { "FOR UPDATE" };
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM {} WHERE credential_id=$1 AND idempotency_key=$2 AND {filter} {lock}",
        columns(scope.workspace().is_some()),
        scope.table()
    )))
    .bind(scope.credential().0)
    .bind(key)
    .bind(scope.workspace())
    .fetch_optional(&mut **tx)
    .await
}
async fn reservation_overdue(
    tx: &mut Transaction<'_, Postgres>,
    workspace: Option<WorkspaceId>,
    operation: OperationId,
) -> Result<bool, sqlx::Error> {
    let Some(w) = workspace else {
        return Ok(true);
    };
    sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM upload_reservations WHERE workspace_id=$1 AND operation_id=$2 AND state='active' AND expires_at>clock_timestamp())")
        .bind(w.0).bind(operation.0).fetch_one(&mut **tx).await
}
async fn classify(
    tx: &mut Transaction<'_, Postgres>,
    workspace: Option<WorkspaceId>,
    row: Stored,
    digest: &[u8; 32],
    tombstoned: Option<Tombstoned>,
) -> Result<ReceiptEntry, sqlx::Error> {
    let operation_id = OperationId(row.id);
    if row.request_digest != digest {
        return Ok(ReceiptEntry::Mismatch { operation_id });
    }
    if row.state == "indeterminate" || is_expired(tx, &row).await? {
        return Ok(ReceiptEntry::Expired {
            operation_id,
            tombstoned,
        });
    }
    match (row.state.as_str(), row.result) {
        ("committed", Some(result)) => Ok(ReceiptEntry::Replay {
            operation_id,
            result,
        }),
        ("failed", _) => Ok(ReceiptEntry::Failed { operation_id }),
        ("pending", _) => Ok(ReceiptEntry::Pending {
            operation_id,
            reservation_overdue: reservation_overdue(tx, workspace, operation_id).await?,
        }),
        _ => Err(sqlx::Error::Protocol(
            "invalid persisted receipt state".into(),
        )),
    }
}
/// Read-only receipt classification. End this transaction before taking any fence locks.
pub async fn peek(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    key: &str,
    digest: [u8; 32],
) -> Result<Option<ReceiptEntry>, sqlx::Error> {
    let Some(row) = existing(tx, scope, key, true).await? else {
        return Ok(None);
    };
    Ok(Some(
        classify(tx, scope.workspace().map(WorkspaceId), row, &digest, None).await?,
    ))
}
async fn entry(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    mut row: Stored,
    digest: &[u8; 32],
) -> Result<ReceiptEntry, sqlx::Error> {
    if row.request_digest != digest {
        return Ok(ReceiptEntry::Mismatch {
            operation_id: OperationId(row.id),
        });
    }
    let workspace = scope.workspace().map(WorkspaceId);
    let tombstoned = tombstone(tx, workspace, &mut row).await?;
    if row.state == "pending"
        && let Some(w) = workspace
    {
        match fence_expired_pending(tx, w, OperationId(row.id)).await? {
            FenceOutcome::Fenced => {
                row.state = "failed".into();
                row.result = Some(serde_json::json!({"code":"reservation_expired"}));
            }
            FenceOutcome::AlreadyResolved(resolved) => row = resolved,
            FenceOutcome::StillActive => (),
        }
    }
    classify(tx, workspace, row, digest, tombstoned).await
}

/// Bound only the insertion wait; restore the job's general lock timeout afterwards.
pub async fn begin(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    request: ReceiptRequest<'_>,
) -> Result<ReceiptEntry, ReceiptError> {
    if let Some(row) = existing(tx, scope, request.key, false).await? {
        return Ok(entry(tx, scope, row, &request.digest).await?);
    }
    let previous: String = sqlx::query_scalar("SHOW lock_timeout")
        .fetch_one(&mut **tx)
        .await?;
    let milliseconds = request.wait_bound.as_millis().max(1).min(i32::MAX as u128);
    sqlx::query("SELECT set_config('lock_timeout', $1, true)")
        .bind(format!("{milliseconds}ms"))
        .execute(&mut **tx)
        .await?;
    let id = OperationId::new();
    let owner = random_token();
    let token = sha256(owner.expose());
    let retention_seconds = request.retention.as_secs_f64();
    let inserted = match scope {
        ReceiptScope::Admin { credential } => sqlx::query_scalar::<_, uuid::Uuid>("INSERT INTO admin_operations (id, credential_id, credential_epoch_at_start, idempotency_key, op_type, request_digest, owner_token, heartbeat_at, state, created_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,clock_timestamp(),'pending',transaction_timestamp(),clock_timestamp()+$8*interval '1 second') ON CONFLICT (credential_id,idempotency_key) DO NOTHING RETURNING id")
            .bind(id.0).bind(credential.0).bind(request.credential_epoch).bind(request.key).bind(request.op_type).bind(request.digest.as_slice()).bind(token.0.as_slice()).bind(retention_seconds).fetch_optional(&mut **tx).await,
        ReceiptScope::Workspace { workspace, credential, session, session_epoch, authorization_epoch, mutation_epoch } => sqlx::query_scalar::<_, uuid::Uuid>("INSERT INTO operations (id, credential_id, credential_epoch_at_start, idempotency_key, op_type, request_digest, owner_token, heartbeat_at, state, created_at, expires_at, workspace_id, client_session_id, session_epoch, authorization_epoch_at_start, mutation_epoch_at_start) VALUES ($1,$2,$3,$4,$5,$6,$7,clock_timestamp(),'pending',transaction_timestamp(),clock_timestamp()+$8*interval '1 second',$9,$10,$11,$12,$13) ON CONFLICT (workspace_id,credential_id,idempotency_key) DO NOTHING RETURNING id")
            .bind(id.0).bind(credential.0).bind(request.credential_epoch).bind(request.key).bind(request.op_type).bind(request.digest.as_slice()).bind(token.0.as_slice()).bind(retention_seconds).bind(workspace.0).bind(session.0).bind(session_epoch).bind(authorization_epoch).bind(mutation_epoch).fetch_optional(&mut **tx).await,
    };
    let inserted = match inserted {
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("55P03") => {
            return Err(ReceiptError::PendingOwner);
        }
        result => result?,
    };
    sqlx::query("SELECT set_config('lock_timeout', $1, true)")
        .bind(previous)
        .execute(&mut **tx)
        .await?;
    if inserted.is_some() {
        return Ok(ReceiptEntry::Fresh {
            operation_id: id,
            owner,
        });
    }
    let row = existing(tx, scope, request.key, false)
        .await?
        .ok_or_else(|| sqlx::Error::Protocol("conflicting receipt missing".into()))?;
    Ok(entry(tx, scope, row, &request.digest).await?)
}
/// Conditional terminal writes cannot overwrite a resolved receipt.
pub async fn commit_result(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    operation: OperationId,
    result: &Value,
) -> Result<bool, sqlx::Error> {
    terminal(tx, scope, operation, "committed", result).await
}
pub async fn fail_result(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    operation: OperationId,
    code: &str,
) -> Result<bool, sqlx::Error> {
    terminal(
        tx,
        scope,
        operation,
        "failed",
        &serde_json::json!({"code":code}),
    )
    .await
}
async fn terminal(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    operation: OperationId,
    state: &str,
    result: &Value,
) -> Result<bool, sqlx::Error> {
    let filter = if scope.workspace().is_some() {
        "workspace_id=$5"
    } else {
        "$5::uuid IS NULL"
    };
    Ok(sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {} SET state=$3,result=$2,committed_at=transaction_timestamp() WHERE id=$1 AND credential_id=$4 AND {filter} AND state='pending'", scope.table())))
        .bind(operation.0).bind(result).bind(state).bind(scope.credential().0).bind(scope.workspace())
        .execute(&mut **tx).await?.rows_affected() == 1)
}
#[derive(Debug, sqlx::FromRow)]
pub struct PendingOperation {
    #[sqlx(rename = "id")]
    pub operation_id: uuid::Uuid,
    pub mutation_epoch_at_start: i64,
    pub authorization_epoch_at_start: i64,
}
pub async fn lock_pending(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    operation: OperationId,
    owner: &Secret<String>,
) -> Result<PendingOperation, ReceiptError> {
    let Some(workspace) = scope.workspace() else {
        return Err(ReceiptError::NotOwner);
    };
    sqlx::query_as("SELECT id,mutation_epoch_at_start,authorization_epoch_at_start FROM operations WHERE workspace_id=$1 AND id=$2 AND credential_id=$3 AND state='pending' AND owner_token=$4 FOR UPDATE")
        .bind(workspace).bind(operation.0).bind(scope.credential().0).bind(sha256(owner.expose()).0.as_slice())
        .fetch_optional(&mut **tx).await?.ok_or(ReceiptError::NotOwner)
}
#[derive(Debug)]
pub enum FenceOutcome {
    Fenced,
    StillActive,
    AlreadyResolved(Stored),
}
/// Does not take actor/session locks: the operation identity and its row lock
/// serialize owners, finalizers and fences. No resolved receipt is overwritten.
pub async fn fence_expired_pending(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    operation: OperationId,
) -> Result<FenceOutcome, sqlx::Error> {
    use crate::{objects, quota, reservations};
    use anchorfs_core::ReservationId;
    let row: Stored = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM operations WHERE workspace_id=$1 AND id=$2 FOR UPDATE",
        columns(true)
    )))
    .bind(workspace.0)
    .bind(operation.0)
    .fetch_one(&mut **tx)
    .await?;
    if row.state != "pending" {
        return Ok(FenceOutcome::AlreadyResolved(row));
    }
    let reservations: Vec<reservations::ReservationRow> = sqlx::query_as("SELECT * FROM upload_reservations WHERE workspace_id=$1 AND operation_id=$2 ORDER BY id FOR UPDATE")
        .bind(workspace.0).bind(operation.0).fetch_all(&mut **tx).await?;
    if !reservation_overdue(tx, Some(workspace), operation).await? {
        return Ok(FenceOutcome::StillActive);
    }
    let mut reserved = 0i64;
    let mut orphans = 0i64;
    for reservation in reservations.iter().filter(|r| r.state == "active") {
        reservations::fail(tx, workspace, reservation, "expired").await?;
        let bytes = objects::orphan(
            tx,
            workspace,
            ReservationId(reservation.id),
            &[],
            &[],
            "reservation_expired",
        )
        .await?;
        reserved = reserved
            .checked_add(reservation.reserved_bytes)
            .ok_or_else(|| sqlx::Error::Protocol("reservation sum overflow".into()))?;
        orphans = orphans
            .checked_add(bytes)
            .ok_or_else(|| sqlx::Error::Protocol("orphan sum overflow".into()))?;
    }
    if reserved > 0 {
        quota::release_to_orphans(tx, workspace, reserved, orphans).await?;
    }
    let updated = sqlx::query("UPDATE operations SET state='failed',result=jsonb_build_object('code','reservation_expired'),committed_at=transaction_timestamp() WHERE workspace_id=$1 AND id=$2 AND state='pending'")
        .bind(workspace.0).bind(operation.0).execute(&mut **tx).await?.rows_affected();
    if updated != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(FenceOutcome::Fenced)
}

/// Inspection scope is separate from mutation scope: hosts may inspect other actors by id.
#[derive(Debug, Clone, Copy)]
pub struct ReceiptFilter {
    pub workspace: Option<WorkspaceId>,
    pub credential: CredentialId,
    pub allow_workspace_actors: bool,
}
/// Inspection chooses exactly one identity; by-key lookup always stays actor-scoped.
pub enum ReceiptLookup {
    Id(OperationId),
    Key(String),
}
pub async fn read(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    lookup: ReceiptLookup,
) -> Result<Option<ReceiptView>, sqlx::Error> {
    let table = if filter.workspace.is_some() {
        "operations"
    } else {
        "admin_operations"
    };
    let workspace_filter = if filter.workspace.is_some() {
        "workspace_id=$1"
    } else {
        "$1::uuid IS NULL"
    };
    let by_id = matches!(&lookup, ReceiptLookup::Id(_));
    let predicate = if by_id { "id=$4" } else { "idempotency_key=$4" };
    // Only fixed enum-selected predicates/table names are interpolated.
    let query = format!(
        "SELECT {} FROM {table} WHERE {workspace_filter} AND (credential_id=$2 OR $3) AND {predicate} FOR UPDATE",
        columns(filter.workspace.is_some())
    );
    let query = sqlx::query_as::<_, Stored>(sqlx::AssertSqlSafe(query))
        .bind(filter.workspace.map(|w| w.0))
        .bind(filter.credential.0)
        .bind(by_id && filter.allow_workspace_actors);
    let row = match lookup {
        ReceiptLookup::Id(id) => query.bind(id.0).fetch_optional(&mut **tx).await?,
        ReceiptLookup::Key(key) => query.bind(key).fetch_optional(&mut **tx).await?,
    };
    let Some(mut row) = row else {
        return Ok(None);
    };
    let tombstoned = tombstone(tx, filter.workspace, &mut row).await?;
    if row.state == "pending"
        && let Some(w) = filter.workspace
    {
        match fence_expired_pending(tx, w, OperationId(row.id)).await? {
            FenceOutcome::Fenced => {
                row.state = "failed".into();
                row.result = Some(serde_json::json!({"code":"reservation_expired"}));
            }
            FenceOutcome::AlreadyResolved(resolved) => row = resolved,
            FenceOutcome::StillActive => (),
        }
    }
    Ok(Some(ReceiptView {
        operation_id: OperationId(row.id),
        state: row.state,
        result: row.result,
        tombstoned,
    }))
}
pub async fn read_by_id(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    id: OperationId,
) -> Result<Option<ReceiptView>, sqlx::Error> {
    read(tx, filter, ReceiptLookup::Id(id)).await
}
pub async fn read_by_key(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    key: &str,
) -> Result<Option<ReceiptView>, sqlx::Error> {
    read(tx, filter, ReceiptLookup::Key(key.to_owned())).await
}
