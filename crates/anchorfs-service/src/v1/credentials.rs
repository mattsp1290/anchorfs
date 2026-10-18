use crate::{
    app::AppState,
    control,
    middleware::{RequestStart, TraceId},
    receipt::{self, Authority, Published},
};
use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_core::{ErrorCode, WorkspaceId};
use anchorfs_persistence::{
    authority::{self, CredentialKind as Kind, CredentialRole as Role, LockMode},
    credentials,
    receipts::{ReceiptRequest, ReceiptScope},
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
        || (kind != Kind::Administrator
            && (body.workspace_id.is_none() || (kind == Kind::Worker && role == Role::Admin)))
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
                let mutation_actor = actor.clone();
                receipt::execute(
                    tx,
                    Authority {
                        actor: &actor,
                        session: None,
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type: "credential_issue",
                        digest,
                        credential_epoch: actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            let issued = credentials::issue(tx, &mutation_actor, new).await?;
                            Published::credential(super::views::credential(operation, issued))
                        })
                    },
                )
                .await
            })
        },
    )
    .await;
    receipt::response(result, StatusCode::CREATED, &trace.0)
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
                let mutation_actor = actor.clone();
                receipt::execute(
                    tx,
                    Authority {
                        actor: &actor,
                        session: None,
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type,
                        digest,
                        credential_epoch: actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            if rotation {
                                let issued =
                                    credentials::rotate(tx, &mutation_actor, target).await?;
                                Published::credential(super::views::credential(operation, issued))
                            } else {
                                credentials::revoke(tx, &mutation_actor, target).await?;
                                Published::metadata(super::views::mutation(operation))
                            }
                        })
                    },
                )
                .await
            })
        },
    )
    .await;
    receipt::response(result, StatusCode::OK, &trace.0)
}
