use crate::{
    app::AppState,
    control,
    middleware::{RequestStart, TraceId},
    receipt::{self, Start},
};
use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_core::{ErrorCode, WorkspaceId};
use anchorfs_persistence::{
    authority::{self, CredentialKind as Kind, CredentialRole as Role, LockMode},
    credentials,
    receipts::{self, ReceiptScope},
};
use axum::{http::StatusCode, response::Response};
use uuid::Uuid;
#[utoipa::path(post,
 path="/v1/credentials",
 operation_id="createCredential",
 request_body = CreateCredential,
 params(("Idempotency-Key" = String,
 Header,
 description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,
description="Committed credential authority change",
body=IssuedCredential),
 (status=404,
description="not_found",
body=ErrorBody),
(status=507,
description="quota_exceeded or capacity_exhausted",
body=ErrorBody),
(status = 501,
 description = "Stage 0 declared operation",
 body = ErrorBody),
(status = 400,
 description = "invalid_input",
 body = ErrorBody),
(status = 401,
 description = "unauthenticated",
 body = ErrorBody),
(status = 403,
 description = "forbidden",
 body = ErrorBody),
(status = 503,
 description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)",
 body = ErrorBody),
(status = 409,
 description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate",
 body = ErrorBody)),
 security(("credential" = [])))]
pub async fn create_credential(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    Bearer(proof): Bearer,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateCredential>,
) -> Result<Response, ApiError> {
    let digest = receipt::request_digest(
        "credential_issue",
        &[],
        Some(&serde_json::to_value(&body).expect("request JSON")),
    );
    let kind = match body.kind {
        CredentialKind::Administrator => Kind::Administrator,
        CredentialKind::Host => Kind::Host,
        CredentialKind::Worker => Kind::Worker,
    };
    let role = match body.role {
        CredentialRole::Read => Role::Read,
        CredentialRole::Write => Role::Write,
        CredentialRole::Admin => Role::Admin,
    };
    let expires_at = body
        .expires_at
        .as_deref()
        .map(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339))
        .transpose()
        .map_err(|_| ApiError::new(ErrorCode::InvalidInput, &trace.0))?;
    if body.max_reservation_bytes == 0
        || body.max_reservation_bytes > i64::MAX as u64
        || (kind == Kind::Administrator && (body.workspace_id.is_some() || role != Role::Admin))
        || (kind != Kind::Administrator && (body.workspace_id.is_none() || role == Role::Admin))
    {
        return Err(ApiError::new(ErrorCode::InvalidInput, trace.0));
    }
    let new = credentials::NewCredential {
        workspace_id: body.workspace_id.map(WorkspaceId),
        kind,
        role,
        uid: body.uid,
        gid: body.gid,
        expires_at,
        max_reservation_bytes: body.max_reservation_bytes as i64,
    };
    let retention = state.config.receipt_retention;
    let result = control::run(
        &state,
        start,
        trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let workspace_exists = match new.workspace_id {
                    Some(id) => authority::lock_workspace(tx, id).await?.is_some(),
                    None => true,
                };
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                if !workspace_exists {
                    return Err(authority::AuthorityError::NotFound.into());
                }
                let scope = ReceiptScope::Admin {
                    credential: actor.id,
                };
                let operation = match receipt::begin(
                    tx,
                    scope,
                    anchorfs_persistence::receipts::ReceiptRequest {
                        key: &key,
                        op_type: "credential_issue",
                        digest,
                        credential_epoch: actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                )
                .await?
                {
                    Start::Fresh(id) => id,
                    Start::Complete(result) => return Ok(result),
                };
                let issued = credentials::issue(tx, &actor, new).await?;
                let stored = issued_json(operation, &issued, false);
                receipts::commit_result(tx, scope, operation, &stored).await?;
                Ok(Ok(issued_json(operation, &issued, true)))
            })
        },
    )
    .await;
    secret_response(result, StatusCode::CREATED, &trace.0)
}
#[utoipa::path(post,
 path="/v1/credentials/{credential_id}/rotate",
 operation_id="rotateCredential",
 params(("credential_id" = Uuid,
 Path),
("Idempotency-Key" = String,
 Header,
 description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,
description="Committed credential authority change",
body=IssuedCredential),
 (status=404,
description="not_found",
body=ErrorBody),
(status=507,
description="quota_exceeded or capacity_exhausted",
body=ErrorBody),
(status = 501,
 description = "Stage 0 declared operation",
 body = ErrorBody),
(status = 400,
 description = "invalid_input",
 body = ErrorBody),
(status = 401,
 description = "unauthenticated",
 body = ErrorBody),
(status = 403,
 description = "forbidden",
 body = ErrorBody),
(status = 503,
 description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)",
 body = ErrorBody),
(status = 409,
 description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate",
 body = ErrorBody)),
 security(("credential" = [])))]
pub async fn rotate_credential(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    ApiPath((target,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate_credential(
        state,
        start,
        trace,
        proof,
        key,
        anchorfs_core::CredentialId(target),
        true,
    )
    .await
}
#[utoipa::path(delete,
 path="/v1/credentials/{credential_id}",
 operation_id="revokeCredential",
 params(("credential_id" = Uuid,
 Path),
("Idempotency-Key" = String,
 Header,
 description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,
description="Committed credential authority change",
body=MutationResult),
 (status=404,
description="not_found",
body=ErrorBody),
(status=507,
description="quota_exceeded or capacity_exhausted",
body=ErrorBody),
(status = 501,
 description = "Stage 0 declared operation",
 body = ErrorBody),
(status = 400,
 description = "invalid_input",
 body = ErrorBody),
(status = 401,
 description = "unauthenticated",
 body = ErrorBody),
(status = 403,
 description = "forbidden",
 body = ErrorBody),
(status = 503,
 description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)",
 body = ErrorBody),
(status = 409,
 description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate",
 body = ErrorBody)),
 security(("credential" = [])))]
pub async fn revoke_credential(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    ApiPath((target,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate_credential(
        state,
        start,
        trace,
        proof,
        key,
        anchorfs_core::CredentialId(target),
        false,
    )
    .await
}
async fn mutate_credential(
    state: AppState,
    start: RequestStart,
    trace: TraceId,
    proof: anchorfs_core::CredentialProof,
    key: String,
    target: anchorfs_core::CredentialId,
    rotation: bool,
) -> Result<Response, ApiError> {
    let op_type = if rotation {
        "credential_rotate"
    } else {
        "credential_revoke"
    };
    let digest = receipt::request_digest(op_type, &[&target.to_string()], None);
    let retention = state.config.receipt_retention;
    let result = control::run(
        &state,
        start,
        trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let workspace: Option<Option<Uuid>> =
                    sqlx::query_scalar("SELECT workspace_id FROM credentials WHERE id=$1")
                        .bind(target.0)
                        .fetch_optional(&mut **tx)
                        .await?;
                if let Some(Some(workspace)) = workspace {
                    authority::lock_workspace(tx, WorkspaceId(workspace)).await?;
                }
                let update = if rotation {
                    target == proof.id
                } else {
                    credentials::is_ancestor_or_self(tx, target, proof.id).await?
                };
                let actor = authority::load_actor(
                    tx,
                    &proof,
                    if update {
                        LockMode::Update
                    } else {
                        LockMode::Share
                    },
                )
                .await?;
                if workspace.is_none() {
                    return Err(authority::AuthorityError::NotFound.into());
                }
                let scope = ReceiptScope::Admin {
                    credential: actor.id,
                };
                let operation = match receipt::begin(
                    tx,
                    scope,
                    anchorfs_persistence::receipts::ReceiptRequest {
                        key: &key,
                        op_type,
                        digest,
                        credential_epoch: actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                )
                .await?
                {
                    Start::Fresh(id) => id,
                    Start::Complete(result) => return Ok(result),
                };
                if rotation {
                    let issued = credentials::rotate(tx, &actor, target).await?;
                    receipts::commit_result(
                        tx,
                        scope,
                        operation,
                        &issued_json(operation, &issued, false),
                    )
                    .await?;
                    Ok(Ok(issued_json(operation, &issued, true)))
                } else {
                    credentials::revoke(tx, &actor, target).await?;
                    let result = serde_json::json!({
                    "operation_id":operation.0,
                    "inode_id":null,
                    "generation":null}
                    );
                    receipts::commit_result(tx, scope, operation, &result).await?;
                    Ok(Ok(result))
                }
            })
        },
    )
    .await;
    secret_response(result, StatusCode::OK, &trace.0)
}
fn issued_json(
    operation: anchorfs_core::OperationId,
    issued: &credentials::Issued,
    expose: bool,
) -> serde_json::Value {
    serde_json::json!({
    "operation_id":operation.0,
    "credential_id":issued.id.0,
    "secret":if expose {
     Some(issued.secret.expose()) }
     else {
     None }
    ,
    "credential_epoch":issued.epoch,
    "expires_at":issued.expires_at.map(control::timestamp)}
    )
}
fn secret_response(
    result: Result<Result<serde_json::Value, ApiError>, ApiError>,
    status: StatusCode,
    trace: &str,
) -> Result<Response, ApiError> {
    let carries_secret = result
        .as_ref()
        .ok()
        .and_then(|v| v.as_ref().ok())
        .is_some_and(|v| v.get("secret").is_some_and(|s| !s.is_null()));
    let mut response = receipt::response(result, status, trace)?;
    if carries_secret {
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    Ok(response)
}
