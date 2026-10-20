use super::validate_path;
use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/read-pins", operation_id="createReadPin",
 request_body = PathBody,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,description="Committed read pin lifecycle",body=ReadPinGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate, operation_failed", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_read_pin(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<PathBody>,
) -> Result<axum::response::Response, ApiError> {
    validate_path(&body.path, &context.trace.0)?;
    execute(
        state,
        context,
        authority,
        anchorfs_core::WorkspaceId(workspace),
        key,
        PinRequest::Create(anchorfs_core::WorkspacePath::parse(body.path).expect("validated path")),
    )
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/read-pins/{read_pin_id}", operation_id="releaseReadPin",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("read_pin_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed read pin lifecycle",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate, operation_failed", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn release_read_pin(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace, pin)): ApiPath<(Uuid, Uuid)>,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<axum::response::Response, ApiError> {
    execute(
        state,
        context,
        authority,
        anchorfs_core::WorkspaceId(workspace),
        key,
        PinRequest::Release(anchorfs_core::ReadPinId(pin)),
    )
    .await
}

enum PinRequest {
    Create(anchorfs_core::WorkspacePath),
    Release(anchorfs_core::ReadPinId),
}
async fn execute(
    state: crate::app::AppState,
    context: RequestContext,
    authority: SessionAuthority,
    workspace: anchorfs_core::WorkspaceId,
    key: String,
    request: PinRequest,
) -> Result<axum::response::Response, ApiError> {
    use crate::{
        control,
        receipt::{self, Published},
    };
    use anchorfs_persistence::{
        authority::LeaseMode,
        manifests, namespace, pins,
        receipts::{ReceiptRequest, ReceiptScope},
    };
    let (op, ids, body, status) = match &request {
        PinRequest::Create(path) => (
            "read_pin_create",
            vec![workspace.to_string()],
            Some(serde_json::json!({"path":path.as_str()})),
            axum::http::StatusCode::CREATED,
        ),
        PinRequest::Release(id) => (
            "read_pin_release",
            vec![workspace.to_string(), id.to_string()],
            None,
            axum::http::StatusCode::OK,
        ),
    };
    let digest = receipt::request_digest(
        op,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        body.as_ref(),
    );
    let config = state.config.clone();
    let result = control::run(
        &state,
        context.start,
        context.trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let p = super::prologue(
                    tx,
                    &authority.proof,
                    workspace,
                    &authority.binding,
                    authority.workflow.as_ref(),
                    super::Gate {
                        role_mutation: false,
                        lease: LeaseMode::Read,
                    },
                )
                .await?;
                let scope = ReceiptScope::Workspace {
                    workspace,
                    credential: p.actor.id,
                    session: p.session.id,
                    session_epoch: p.session.session_epoch,
                    authorization_epoch: p.workspace.authorization_epoch,
                    mutation_epoch: p.workspace.mutation_epoch,
                };
                let session = p.session.clone();
                receipt::execute(
                    tx,
                    receipt::Authority {
                        actor: &p.actor,
                        session: Some(&p.session),
                        workflow: Some(receipt::WorkflowAuthority {
                            workspace: &p.workspace,
                            proof: authority.workflow.as_ref(),
                            mode: LeaseMode::Read,
                        }),
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type: op,
                        digest,
                        credential_epoch: p.actor.epoch,
                        retention: config.receipt_retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            match request {
                                PinRequest::Create(path) => {
                                    let inode =
                                        namespace::resolve(tx, workspace, Some(&path), &p.caller)
                                            .await?;
                                    if inode.kind != "file" {
                                        return Err(ApiError::new(
                                            if inode.kind == "directory" {
                                                anchorfs_core::ErrorCode::IsDirectory
                                            } else {
                                                anchorfs_core::ErrorCode::InvalidInput
                                            },
                                            "",
                                        ));
                                    }
                                    namespace::access(
                                        &inode,
                                        &p.caller,
                                        anchorfs_core::Access {
                                            read: true,
                                            write: false,
                                            exec: false,
                                        },
                                    )?;
                                    let manifest =
                                        manifests::current(tx, workspace, &inode).await?;
                                    let pin = pins::create(
                                        tx,
                                        workspace,
                                        &session,
                                        anchorfs_core::ManifestId(manifest.id),
                                        config.read_pin_ttl,
                                        config.max_read_pins_per_session,
                                    )
                                    .await?;
                                    Published::metadata(ReadPinGrant {
                                        operation_id: operation.0,
                                        read_pin_id: pin.id,
                                        manifest_id: manifest.id,
                                        generation: manifest.generation,
                                        size: manifest.size as u64,
                                        expires_at: control::timestamp(pin.expires_at),
                                    })
                                }
                                PinRequest::Release(id) => {
                                    pins::release(tx, workspace, &session, id).await?;
                                    Published::metadata(MutationResult {
                                        operation_id: operation.0,
                                        inode_id: None,
                                        generation: None,
                                    })
                                }
                            }
                        })
                    },
                )
                .await
            })
        },
    )
    .await;
    receipt::finish(&state, result, status, &context.trace.0).await
}
