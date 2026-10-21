use crate::{
    app::AppState,
    control,
    dto::*,
    error::ApiError,
    extract::*,
    middleware::{RequestStart, TraceId},
    receipt::{self, Authority, Published},
};
use anchorfs_core::{CredentialProof, ErrorCode, SessionId, WorkspaceId};
use anchorfs_persistence::{
    authority::{self, LockMode, WorkflowProof},
    receipts::{ReceiptRequest, ReceiptScope},
    workflows,
};
use axum::{extract::State, http::StatusCode, response::Response};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/workflow-lease", operation_id="acquireWorkflowLease",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="must be absent; a lease is being created, not proved"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1, description="must be absent; a lease is being created, not proved"),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,description="Committed workflow authority change",body=WorkflowGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "workspace_busy, fenced, idempotency_mismatch, operation_indeterminate, operation_failed", body = ErrorBody)), security(("credential" = [])))]
pub async fn acquire_workflow_lease(
    State(state): State<AppState>,
    RequestContext { trace, start }: RequestContext,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
    OptionalWorkflowGrant(grant): OptionalWorkflowGrant,
) -> Result<Response, ApiError> {
    if grant.is_some() {
        let mut error = ApiError::new(ErrorCode::InvalidInput, &trace.0);
        error.message = "workflow_proof_not_applicable".into();
        return Err(error);
    }
    mutate(LeaseMutation {
        state,
        trace,
        start,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        kind: Mutation::Acquire,
    })
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/workflow-lease/renew", operation_id="renewWorkflowLease",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = String, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = i64, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed workflow authority change",body=WorkflowGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "workspace_busy, fenced, idempotency_mismatch, operation_indeterminate, operation_failed", body = ErrorBody)), security(("credential" = [])))]
pub async fn renew_workflow_lease(
    State(state): State<AppState>,
    RequestContext { trace, start }: RequestContext,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    RequiredWorkflowGrant(grant): RequiredWorkflowGrant,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate(LeaseMutation {
        state,
        trace,
        start,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        kind: Mutation::Renew(grant),
    })
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/workflow-lease", operation_id="releaseWorkflowLease",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = String, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = i64, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed workflow authority change",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "workspace_busy, fenced, idempotency_mismatch, operation_indeterminate, operation_failed", body = ErrorBody)), security(("credential" = [])))]
pub async fn release_workflow_lease(
    State(state): State<AppState>,
    RequestContext { trace, start }: RequestContext,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    RequiredWorkflowGrant(grant): RequiredWorkflowGrant,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
) -> Result<Response, ApiError> {
    mutate(LeaseMutation {
        state,
        trace,
        start,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        kind: Mutation::Release(grant),
    })
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/workflow-lease/takeover", operation_id="takeoverWorkflowLease",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="must be absent; a lease is being created, not proved"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1, description="must be absent; a lease is being created, not proved"),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=201,description="Committed workflow authority change",body=WorkflowGrant), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "workspace_busy, fenced, idempotency_mismatch, operation_indeterminate, operation_failed", body = ErrorBody)), security(("credential" = [])))]
pub async fn takeover_workflow_lease(
    State(state): State<AppState>,
    RequestContext { trace, start }: RequestContext,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
    session: SessionBinding,
    IdempotencyKey(key): IdempotencyKey,
    OptionalWorkflowGrant(grant): OptionalWorkflowGrant,
) -> Result<Response, ApiError> {
    if grant.is_some() {
        let mut error = ApiError::new(ErrorCode::InvalidInput, &trace.0);
        error.message = "workflow_proof_not_applicable".into();
        return Err(error);
    }
    mutate(LeaseMutation {
        state,
        trace,
        start,
        proof,
        session,
        key,
        workspace: WorkspaceId(workspace),
        kind: Mutation::Takeover,
    })
    .await
}

enum Mutation {
    Acquire,
    Renew(WorkflowProof),
    Release(WorkflowProof),
    Takeover,
}
impl Mutation {
    fn op_type(&self) -> &'static str {
        match self {
            Self::Acquire => "workflow_acquire",
            Self::Renew(_) => "workflow_renew",
            Self::Release(_) => "workflow_release",
            Self::Takeover => "workflow_takeover",
        }
    }
}
struct LeaseMutation {
    state: AppState,
    trace: TraceId,
    start: RequestStart,
    proof: CredentialProof,
    session: SessionBinding,
    key: String,
    workspace: WorkspaceId,
    kind: Mutation,
}
async fn mutate(request: LeaseMutation) -> Result<Response, ApiError> {
    let LeaseMutation {
        state,
        trace,
        start,
        proof,
        session,
        key,
        workspace,
        kind,
    } = request;
    let op_type = kind.op_type();
    let epoch = match &kind {
        Mutation::Renew(p) | Mutation::Release(p) => Some(p.epoch.to_string()),
        _ => None,
    };
    let ws = workspace.to_string();
    let mut ids = vec![ws.as_str()];
    if let Some(e) = &epoch {
        ids.push(e);
    }
    let digest = receipt::request_digest(op_type, &ids, None);
    let status = if matches!(kind, Mutation::Acquire | Mutation::Takeover) {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let ttl = state.config.workflow_lease_ttl;
    let retention = state.config.receipt_retention;
    let lock_timeout = state.config.db_lock_timeout;
    let result = control::run(
        &state,
        start,
        trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                authority::preauthenticate(tx, &proof, workspace).await?;
                // Bound exclusive waits by the response budget: abandoned lock waiters
                // must not later acquire and commit a grant whose token was never seen.
                let millis = lock_wait_millis(lock_timeout, budget.remaining());
                sqlx::query("SELECT set_config('lock_timeout', $1, true)")
                    .bind(format!("{millis}ms"))
                    .execute(&mut **tx)
                    .await?;
                let lock = authority::lock_workspace_for_update(tx, workspace)
                    .await?
                    .ok_or(authority::AuthorityError::NotFound)?;
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                let session = authority::load_actor_session(
                    tx,
                    &actor,
                    workspace,
                    SessionId(session.session_id),
                    session.session_epoch,
                    LockMode::Share,
                )
                .await?;
                if session.bound_mutation_epoch.is_some() {
                    return Err(workflows::WorkflowError::SessionBound.into());
                }
                let scope = ReceiptScope::Workspace {
                    workspace,
                    credential: actor.id,
                    session: session.id,
                    session_epoch: session.session_epoch,
                    authorization_epoch: lock.authorization_epoch,
                    mutation_epoch: lock.mutation_epoch,
                };
                let mutation_actor = actor.clone();
                let mutation_session = session.clone();
                receipt::execute(
                    tx,
                    Authority {
                        actor: &actor,
                        session: Some(&session),
                        workflow: None,
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
                            match kind {
                                Mutation::Acquire => {
                                    Published::workflow_grant(super::views::workflow_grant(
                                        operation,
                                        workflows::acquire(
                                            tx,
                                            &lock,
                                            &mutation_actor,
                                            &mutation_session,
                                            ttl,
                                        )
                                        .await?,
                                    ))
                                }
                                Mutation::Takeover => {
                                    Published::workflow_grant(super::views::workflow_grant(
                                        operation,
                                        workflows::takeover(
                                            tx,
                                            &lock,
                                            &mutation_actor,
                                            &mutation_session,
                                            ttl,
                                        )
                                        .await?,
                                    ))
                                }
                                Mutation::Renew(proof) => {
                                    Published::metadata(super::views::workflow_renewed(
                                        operation,
                                        workflows::renew(
                                            tx,
                                            &lock,
                                            &mutation_actor,
                                            &mutation_session,
                                            &proof,
                                            ttl,
                                        )
                                        .await?,
                                    ))
                                }
                                Mutation::Release(proof) => {
                                    workflows::release(
                                        tx,
                                        &lock,
                                        &mutation_actor,
                                        &mutation_session,
                                        &proof,
                                    )
                                    .await?;
                                    Published::metadata(super::views::mutation(operation))
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
    receipt::finish(&state, result, status, &trace.0).await
}
fn lock_wait_millis(configured: std::time::Duration, remaining: std::time::Duration) -> u128 {
    configured
        .min(remaining)
        .as_millis()
        .clamp(1, i32::MAX as u128)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_wait_is_bounded_and_never_zero() {
        use std::time::Duration;
        assert_eq!(
            lock_wait_millis(Duration::from_secs(5), Duration::from_millis(200)),
            200
        );
        assert_eq!(
            lock_wait_millis(Duration::from_millis(10), Duration::from_secs(1)),
            10
        );
        assert_eq!(lock_wait_millis(Duration::ZERO, Duration::ZERO), 1);
    }
}
