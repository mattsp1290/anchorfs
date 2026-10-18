use crate::{
    app::AppState,
    control,
    middleware::{RequestStart, TraceId},
    receipt::{self, Start},
};
use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_core::{CredentialId, OperationId, SessionId, WorkspaceId};
use anchorfs_persistence::{
    authority::{self, LockMode},
    receipts::{self, ReceiptRequest, ReceiptScope},
    sessions,
};
use axum::{http::StatusCode, response::Response};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/sessions", operation_id="createSession",
 request_body = CreateSession,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,description="Committed session authority change",body=SessionGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_session(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateSession>,
) -> Result<Response, ApiError> {
    if body.bind_to_workflow_lease.is_some() {
        return Err(ApiError::not_implemented(trace.0));
    }
    let workspace = WorkspaceId(workspace);
    let digest = receipt::request_digest(
        "session_create",
        &[&workspace.to_string()],
        Some(&serde_json::to_value(&body).expect("request JSON")),
    );
    let (ttl, limit, retention) = (
        state.config.client_session_ttl,
        state.config.max_sessions_per_credential,
        state.config.receipt_retention,
    );
    let result = control::run(
        &state,
        start,
        trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let exists = authority::lock_workspace(tx, workspace).await?.is_some();
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                if !exists || !authority::same_workspace(&actor, workspace) {
                    return Err(authority::AuthorityError::NotFound.into());
                }
                let scope = ReceiptScope::Admin {
                    credential: actor.id,
                };
                let operation = match receipt::begin(
                    tx,
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type: "session_create",
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
                let session = sessions::create(
                    tx,
                    &actor,
                    workspace,
                    CredentialId(body.credential_id),
                    ttl,
                    limit,
                )
                .await?;
                let result = grant(operation, session);
                receipts::commit_result(tx, scope, operation, &result).await?;
                Ok(Ok(result))
            })
        },
    )
    .await;
    receipt::response(result, StatusCode::CREATED, &trace.0)
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/sessions/{session_id}/renew", operation_id="renewSession",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("session_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed session authority change",body=SessionGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn renew_session(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    ApiPath((workspace, target)): ApiPath<(Uuid, Uuid)>,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate(SessionMutation {
        state,
        start,
        trace,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        target: SessionId(target),
        kind: Mutation::Renew,
        body: None,
    })
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/sessions/{session_id}/rebind", operation_id="rebindSession",
 request_body = RebindSession,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("session_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed session authority change",body=SessionGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn rebind_session(
    axum::extract::State(state): axum::extract::State<AppState>,
    RequestContext { trace, start }: RequestContext,
    ApiPath((workspace, target)): ApiPath<(Uuid, Uuid)>,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<RebindSession>,
) -> Result<Response, ApiError> {
    if body.workflow_lease_token.is_some() {
        return Err(ApiError::not_implemented(trace.0));
    }
    mutate(SessionMutation {
        state,
        start,
        trace,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        target: SessionId(target),
        kind: Mutation::Rebind,
        body: Some(serde_json::to_value(body).expect("request JSON")),
    })
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/sessions/{session_id}", operation_id="revokeSession",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("session_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed session authority change",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn revoke_session(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Extension(trace): axum::Extension<TraceId>,
    axum::Extension(start): axum::Extension<RequestStart>,
    ApiPath((workspace, target)): ApiPath<(Uuid, Uuid)>,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate(SessionMutation {
        state,
        start,
        trace,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        target: SessionId(target),
        kind: Mutation::Revoke,
        body: None,
    })
    .await
}

#[derive(Clone, Copy)]
enum Mutation {
    Renew,
    Rebind,
    Revoke,
}
impl Mutation {
    fn op_type(self) -> &'static str {
        match self {
            Self::Renew => "session_renew",
            Self::Rebind => "session_rebind",
            Self::Revoke => "session_revoke",
        }
    }
}
struct SessionMutation {
    state: AppState,
    start: RequestStart,
    trace: TraceId,
    proof: anchorfs_core::CredentialProof,
    session: SessionBinding,
    key: String,
    workspace: WorkspaceId,
    target: SessionId,
    kind: Mutation,
    body: Option<serde_json::Value>,
}
async fn mutate(request: SessionMutation) -> Result<Response, ApiError> {
    let SessionMutation {
        state,
        start,
        trace,
        proof,
        session,
        key,
        workspace,
        target,
        kind,
        body,
    } = request;
    let op_type = kind.op_type();
    let digest = receipt::request_digest(
        op_type,
        &[&workspace.to_string(), &target.to_string()],
        body.as_ref(),
    );
    let (ttl, retention) = (
        state.config.client_session_ttl,
        state.config.receipt_retention,
    );
    let result=control::run(&state,start,trace.clone(),control::Kind::Mutation,move |tx,budget| Box::pin(async move {
        let workspace_lock=authority::lock_workspace(tx,workspace).await?;
        let actor=authority::load_actor(tx,&proof,LockMode::Share).await?;
        let mode=if session.session_id==target.0 { LockMode::Update } else { LockMode::Share };
        let actor_session=authority::load_actor_session(tx,&actor,workspace,SessionId(session.session_id),session.session_epoch,mode).await?;
        let workspace_lock=workspace_lock.ok_or(authority::AuthorityError::NotFound)?;
        let scope=ReceiptScope::Workspace { workspace,credential:actor.id,session:actor_session.id,session_epoch:actor_session.session_epoch,authorization_epoch:workspace_lock.authorization_epoch,mutation_epoch:workspace_lock.mutation_epoch };
        let operation=match receipt::begin(tx,scope,ReceiptRequest {key:&key,op_type,digest,credential_epoch:actor.epoch,retention,wait_bound:budget.remaining()}).await? { Start::Fresh(id)=>id,Start::Complete(result)=>return Ok(result) };
        let result=match kind {
            Mutation::Renew=>grant(operation,sessions::renew(tx,&actor,&actor_session,workspace,target,ttl).await?),
            Mutation::Rebind=>grant(operation,sessions::rebind(tx,&actor,&actor_session,workspace,target,ttl).await?),
            Mutation::Revoke=>{ sessions::revoke(tx,&actor,&actor_session,workspace,target).await?; serde_json::json!({"operation_id":operation.0,"inode_id":null,"generation":null}) },
        };
        receipts::commit_result(tx,scope,operation,&result).await?;
        Ok(Ok(result))
    })).await;
    receipt::response(result, StatusCode::OK, &trace.0)
}
fn grant(operation: OperationId, session: sessions::SessionView) -> serde_json::Value {
    serde_json::json!({"operation_id":operation.0,"session_id":session.id.0,"session_epoch":session.session_epoch,"credential_epoch":session.credential_epoch,"bound_mutation_epoch":session.bound_mutation_epoch,"expires_at":control::timestamp(session.expires_at)})
}
