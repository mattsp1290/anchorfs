// SQL audit: interpolated identifiers/clauses come only from private enum/static choices.
// All request data uses bind parameters.
//! Receipts share their transaction with publication; no pending row is committed.
use anchorfs_core::{CredentialId, OperationId, SessionId, WorkspaceId, random_token, sha256};
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
    Fresh(OperationId),
    Replay {
        operation_id: OperationId,
        result: Value,
    },
    Mismatch {
        operation_id: OperationId,
    },
    Expired {
        operation_id: OperationId,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("pending receipt owner")]
    PendingOwner,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
#[derive(sqlx::FromRow)]
struct Stored {
    id: uuid::Uuid,
    request_digest: Vec<u8>,
    state: String,
    result: Option<Value>,
    expires_at: OffsetDateTime,
}
#[derive(Debug)]
pub struct ReceiptView {
    pub operation_id: OperationId,
    pub state: String,
    pub result: Option<Value>,
}

async fn tombstone(
    tx: &mut Transaction<'_, Postgres>,
    table: &str,
    row: &mut Stored,
) -> Result<(), sqlx::Error> {
    let expired: bool = sqlx::query_scalar("SELECT $1::timestamptz <= clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await?;
    if expired && row.state != "indeterminate" {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET state='indeterminate', result=NULL WHERE id=$1"
        )))
        .bind(row.id)
        .execute(&mut **tx)
        .await?;
        row.state = "indeterminate".into();
        row.result = None;
    }
    Ok(())
}
async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    key: &str,
) -> Result<Option<Stored>, sqlx::Error> {
    // $3 always has an explicit type, even in the administrator table branch.
    let filter = if scope.workspace().is_some() {
        "workspace_id=$3"
    } else {
        "$3::uuid IS NULL"
    };
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT id, request_digest, state, result, expires_at FROM {} WHERE credential_id=$1 AND idempotency_key=$2 AND {filter} FOR UPDATE", scope.table())))
        .bind(scope.credential().0).bind(key).bind(scope.workspace()).fetch_optional(&mut **tx).await
}
async fn entry(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    mut row: Stored,
    digest: &[u8; 32],
) -> Result<ReceiptEntry, sqlx::Error> {
    let operation_id = OperationId(row.id);
    if row.request_digest != digest {
        return Ok(ReceiptEntry::Mismatch { operation_id });
    }
    tombstone(tx, scope.table(), &mut row).await?;
    if row.state == "indeterminate" {
        return Ok(ReceiptEntry::Expired { operation_id });
    }
    match (row.state.as_str(), row.result) {
        ("committed", Some(result)) => Ok(ReceiptEntry::Replay {
            operation_id,
            result,
        }),
        // A visible pending/failed row is outside this milestone's publication model.
        _ => Err(sqlx::Error::Protocol(
            "invalid persisted receipt state".into(),
        )),
    }
}

/// Bound only the insertion wait; restore the job's general lock timeout afterwards.
pub async fn begin(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    request: ReceiptRequest<'_>,
) -> Result<ReceiptEntry, ReceiptError> {
    if let Some(row) = existing(tx, scope, request.key).await? {
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
    let token = sha256(random_token().expose());
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
        return Ok(ReceiptEntry::Fresh(id));
    }
    let row = existing(tx, scope, request.key)
        .await?
        .ok_or_else(|| sqlx::Error::Protocol("conflicting receipt missing".into()))?;
    Ok(entry(tx, scope, row, &request.digest).await?)
}
pub async fn commit_result(
    tx: &mut Transaction<'_, Postgres>,
    scope: ReceiptScope,
    operation: OperationId,
    result: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {} SET state='committed', result=$2, committed_at=transaction_timestamp() WHERE id=$1", scope.table())))
        .bind(operation.0).bind(result).execute(&mut **tx).await?;
    Ok(())
}

/// Inspection scope is separate from mutation scope: hosts may inspect other actors by id.
#[derive(Debug, Clone, Copy)]
pub struct ReceiptFilter {
    pub workspace: Option<WorkspaceId>,
    pub credential: CredentialId,
    pub allow_workspace_actors: bool,
}
async fn read(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    id: Option<OperationId>,
    key: Option<&str>,
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
    let query = format!(
        "SELECT id, request_digest, state, result, expires_at FROM {table} WHERE {workspace_filter} AND (credential_id=$2 OR ($3 AND $4::uuid IS NOT NULL)) AND (($4::uuid IS NOT NULL AND id=$4) OR ($4 IS NULL AND idempotency_key=$5)) FOR UPDATE"
    );
    let Some(mut row) = sqlx::query_as::<_, Stored>(sqlx::AssertSqlSafe(query))
        .bind(filter.workspace.map(|w| w.0))
        .bind(filter.credential.0)
        .bind(filter.allow_workspace_actors)
        .bind(id.map(|i| i.0))
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Ok(None);
    };
    tombstone(tx, table, &mut row).await?;
    Ok(Some(ReceiptView {
        operation_id: OperationId(row.id),
        state: row.state,
        result: row.result,
    }))
}
pub async fn read_by_id(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    id: OperationId,
) -> Result<Option<ReceiptView>, sqlx::Error> {
    read(tx, filter, Some(id), None).await
}
pub async fn read_by_key(
    tx: &mut Transaction<'_, Postgres>,
    filter: ReceiptFilter,
    key: &str,
) -> Result<Option<ReceiptView>, sqlx::Error> {
    read(tx, filter, None, Some(key)).await
}
