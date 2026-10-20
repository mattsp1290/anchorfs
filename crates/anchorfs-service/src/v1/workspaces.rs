use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(post,
 path="/v1/workspaces",
 operation_id="createWorkspace",
 request_body = CreateWorkspace,
 params(("Idempotency-Key" = String,
 Header,
 description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,
description="Committed workspace metadata",
body=CreatedWorkspace),
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
pub async fn create_workspace(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    Bearer(proof): Bearer,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> Result<axum::response::Response, ApiError> {
    use crate::{
        control::{self, Kind},
        receipt::{self, Authority, Published},
    };
    use anchorfs_persistence::{
        authority::{self, CredentialKind, LockMode},
        receipts::{ReceiptRequest, ReceiptScope},
        workspaces,
    };
    let invalid = || ApiError::new(anchorfs_core::ErrorCode::InvalidInput, &trace.0);
    let limits = [
        body.logical_bytes_limit,
        body.logical_inodes_limit,
        body.physical_bytes_limit,
        body.staging_bytes_limit,
    ];
    if limits.iter().any(|v| *v == 0 || *v > i64::MAX as u64)
        || body.root_mode.is_some_and(|m| m > 0o777)
    {
        return Err(invalid());
    }
    let digest = receipt::request_digest(
        "workspace_create",
        &[],
        Some(&serde_json::to_value(&body).expect("request JSON")),
    );
    let retention = state.config.receipt_retention;
    let result = control::run(
        &state,
        start,
        trace.clone(),
        Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                if actor.kind != CredentialKind::Administrator {
                    return Err(authority::AuthorityError::Forbidden.into());
                }
                let scope = ReceiptScope::Admin {
                    credential: actor.id,
                };
                receipt::execute(
                    tx,
                    Authority {
                        workflow: None,
                        actor: &actor,
                        session: None,
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type: "workspace_create",
                        digest,
                        credential_epoch: actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            let workspace = workspaces::create(
                                tx,
                                workspaces::NewWorkspace {
                                    logical_bytes_limit: body.logical_bytes_limit as i64,
                                    logical_inodes_limit: body.logical_inodes_limit as i64,
                                    physical_bytes_limit: body.physical_bytes_limit as i64,
                                    staging_bytes_limit: body.staging_bytes_limit as i64,
                                    root_uid: body.root_uid,
                                    root_gid: body.root_gid,
                                    root_mode: body.root_mode.unwrap_or(0o755),
                                },
                            )
                            .await?;
                            Published::metadata(CreatedWorkspace {
                                operation_id: operation.0,
                                workspace: super::views::workspace(workspace)?,
                            })
                        })
                    },
                )
                .await
            })
        },
    )
    .await;
    receipt::finish(&state, result, axum::http::StatusCode::CREATED, &trace.0).await
}
#[utoipa::path(get,
 path="/v1/workspaces/{workspace_id}",
 operation_id="getWorkspace",
 params(("workspace_id" = Uuid,
 Path)),
 responses((status=200,
description="Committed workspace metadata",
body=WorkspaceInfo),
 (status=404,
description="not_found",
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
 body = ErrorBody)),
 security(("credential" = [])))]
pub async fn get_workspace(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
) -> Result<axum::response::Response, ApiError> {
    use anchorfs_persistence::{
        authority::{self, LockMode},
        workspaces,
    };
    let result = crate::control::run(
        &state,
        start,
        trace.clone(),
        crate::control::Kind::Read,
        move |tx, _| {
            Box::pin(async move {
                let workspace = anchorfs_core::WorkspaceId(workspace);
                let locked = authority::lock_workspace(tx, workspace).await?;
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                if locked.is_none() || !authority::same_workspace(&actor, workspace) {
                    return Err(authority::AuthorityError::NotFound.into());
                }
                let view = workspaces::read(tx, workspace)
                    .await?
                    .ok_or(authority::AuthorityError::NotFound)?;
                authority::check_publication_time(tx, &actor, None).await?;
                Ok(Ok(crate::receipt::Answer::metadata(
                    super::views::workspace(view)?,
                )?))
            })
        },
    )
    .await;
    crate::receipt::finish(&state, result, axum::http::StatusCode::OK, &trace.0).await
}
