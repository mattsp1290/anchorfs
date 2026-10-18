use super::validate_path;
use crate::{
    app::AppState,
    control,
    receipt::{self, Authority, Published},
};
use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_core::{HandleId, WorkspaceId, WorkspacePath};
use anchorfs_persistence::{
    authority::LeaseMode,
    handles, namespace,
    receipts::{ReceiptRequest, ReceiptScope},
};
use axum::{extract::State, http::StatusCode, response::Response};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles", operation_id="openHandle",
 request_body = OpenHandle,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,description="Committed handle lifecycle",body=HandleGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn open_handle(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<OpenHandle>,
) -> Result<Response, ApiError> {
    validate_path(&body.path, &context.trace.0)?;
    handles::validate_request(&handles::OpenRequest {
        path: WorkspacePath::parse(&body.path).expect("validated path"),
        read: body.read,
        write: body.write,
        create: body.create,
        exclusive: body.exclusive,
        truncate: body.truncate,
        mode: body.mode,
    })?;
    execute(
        HandleCall {
            state,
            context,
            authority,
            workspace: WorkspaceId(workspace),
            key,
        },
        HandleRequest::Open(body),
    )
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/renew", operation_id="renewHandle",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed handle lifecycle",body=HandleGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn renew_handle(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace, handle)): ApiPath<(Uuid, Uuid)>,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    execute(
        HandleCall {
            state,
            context,
            authority,
            workspace: WorkspaceId(workspace),
            key,
        },
        HandleRequest::Renew(HandleId(handle)),
    )
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/handles/{handle_id}", operation_id="closeHandle",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed handle lifecycle",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn close_handle(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace, handle)): ApiPath<(Uuid, Uuid)>,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    execute(
        HandleCall {
            state,
            context,
            authority,
            workspace: WorkspaceId(workspace),
            key,
        },
        HandleRequest::Close(HandleId(handle)),
    )
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/truncate", operation_id="truncate",
 request_body = Truncate,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Future success contract; Stage 0 returns 501",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn truncate(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    _session: SessionBinding,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<Truncate>,
) -> Result<(), ApiError> {
    let _ = body;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/fsync", operation_id="fsync",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Future success contract; Stage 0 returns 501",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn fsync(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    _session: SessionBinding,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    _key: IdempotencyKey,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}

struct HandleCall {
    state: AppState,
    context: RequestContext,
    authority: SessionAuthority,
    workspace: WorkspaceId,
    key: String,
}
enum HandleRequest {
    Open(OpenHandle),
    Renew(HandleId),
    Close(HandleId),
}
impl HandleRequest {
    fn mutation_gate(&self) -> bool {
        matches!(self,Self::Open(b) if b.write || b.create || b.truncate)
    }
    fn digest(&self, w: WorkspaceId) -> (&'static str, [u8; 32]) {
        let (op, ids, body) = match self {
            Self::Open(b) => (
                "handle_open",
                vec![w.to_string()],
                Some(serde_json::to_value(b).expect("request JSON")),
            ),
            Self::Renew(id) => ("handle_renew", vec![w.to_string(), id.to_string()], None),
            Self::Close(id) => ("handle_close", vec![w.to_string(), id.to_string()], None),
        };
        (
            op,
            receipt::request_digest(
                op,
                &ids.iter().map(String::as_str).collect::<Vec<_>>(),
                body.as_ref(),
            ),
        )
    }
}
async fn execute(call: HandleCall, request: HandleRequest) -> Result<Response, ApiError> {
    let HandleCall {
        state,
        context,
        authority,
        workspace,
        key,
    } = call;
    let (op_type, digest) = request.digest(workspace);
    let status = if matches!(request, HandleRequest::Open(_)) {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let mutation = request.mutation_gate();
    let retention = state.config.receipt_retention;
    let policy = handles::HandlePolicy {
        ttl: state.config.handle_ttl,
        limit: state.config.max_handles_per_session,
    };
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
                        role_mutation: mutation,
                        lease: if mutation {
                            LeaseMode::Mutation
                        } else {
                            LeaseMode::Read
                        },
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
                    Authority {
                        actor: &p.actor,
                        session: Some(&p.session),
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type,
                        digest,
                        credential_epoch: p.actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            match request {
                                HandleRequest::Open(body) => {
                                    let guard = if body.create {
                                        Some(namespace::lock(tx, workspace).await?)
                                    } else {
                                        None
                                    };
                                    let req = handles::OpenRequest {
                                        path: WorkspacePath::parse(body.path)
                                            .expect("validated path"),
                                        read: body.read,
                                        write: body.write,
                                        create: body.create,
                                        exclusive: body.exclusive,
                                        truncate: body.truncate,
                                        mode: body.mode,
                                    };
                                    let opened = handles::open_file(
                                        tx,
                                        guard.as_ref(),
                                        &p.caller,
                                        &session,
                                        req,
                                        policy,
                                    )
                                    .await?;
                                    Published::metadata(super::views::handle(
                                        operation,
                                        &opened.handle,
                                        &opened.inode,
                                    )?)
                                }
                                HandleRequest::Renew(id) => {
                                    let (handle, inode) =
                                        handles::renew(tx, workspace, &session, id, policy.ttl)
                                            .await?;
                                    Published::metadata(super::views::handle(
                                        operation, &handle, &inode,
                                    )?)
                                }
                                HandleRequest::Close(id) => {
                                    let (_, inode) =
                                        handles::close(tx, workspace, &session, id).await?;
                                    Published::metadata(MutationResult {
                                        operation_id: operation.0,
                                        inode_id: Some(inode.id),
                                        generation: Some(inode.generation),
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
    receipt::response(result, status, &context.trace.0)
}
