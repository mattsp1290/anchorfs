use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_persistence::receipts::ReceiptLookup;
use uuid::Uuid;
#[utoipa::path(get,
 path="/v1/workspaces/{workspace_id}/operations/{operation_id}",
 operation_id="getOperation",
 params(("workspace_id" = Uuid,
 Path),
("operation_id" = Uuid,
 Path)),
 responses((status=200,
description="Committed workspace receipt or expired tombstone",
body=OperationStatus),
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
pub async fn get_operation(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    ApiPath((workspace, operation)): ApiPath<(Uuid, Uuid)>,
    Bearer(proof): Bearer,
) -> Result<axum::response::Response, ApiError> {
    inspect(
        state,
        start,
        trace,
        proof,
        Some(anchorfs_core::WorkspaceId(workspace)),
        ReceiptLookup::Id(anchorfs_core::OperationId(operation)),
    )
    .await
}
#[utoipa::path(get,
 path="/v1/workspaces/{workspace_id}/operations",
 operation_id="findOperation",
 params(("workspace_id" = Uuid,
 Path),
("idempotency_key" = String,
 Query)),
 responses((status=200,
description="Committed workspace receipt or expired tombstone",
body=OperationStatus),
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
pub async fn find_operation(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<OperationQuery>,
    Bearer(proof): Bearer,
) -> Result<axum::response::Response, ApiError> {
    inspect(
        state,
        start,
        trace,
        proof,
        Some(anchorfs_core::WorkspaceId(workspace)),
        ReceiptLookup::Key(query.idempotency_key),
    )
    .await
}
#[utoipa::path(get,
path="/v1/admin-operations/{operation_id}",
operation_id="getAdminOperation",
params(("operation_id"=Uuid,
Path)),
responses((status=200,
description="Credential-scoped receipt, including expired tombstone",
body=OperationStatus),
(status=400,
description="invalid_input",
body=ErrorBody),
(status=401,
description="unauthenticated",
body=ErrorBody),
(status=403,
description="forbidden",
body=ErrorBody),
(status=404,
description="not_found",
body=ErrorBody),
(status=501,
description="Stage 0 declared operation",
body=ErrorBody)),
security(("credential"=[])))]
pub async fn get_admin_operation(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    ApiPath((operation,)): ApiPath<(Uuid,)>,
    Bearer(proof): Bearer,
) -> Result<axum::response::Response, ApiError> {
    inspect(
        state,
        start,
        trace,
        proof,
        None,
        ReceiptLookup::Id(anchorfs_core::OperationId(operation)),
    )
    .await
}
#[utoipa::path(get,
path="/v1/admin-operations",
operation_id="findAdminOperation",
params(("idempotency_key"=String,
Query)),
responses((status=200,
description="Credential-scoped receipt, including expired tombstone",
body=OperationStatus),
(status=400,
description="invalid_input",
body=ErrorBody),
(status=401,
description="unauthenticated",
body=ErrorBody),
(status=403,
description="forbidden",
body=ErrorBody),
(status=404,
description="not_found",
body=ErrorBody),
(status=501,
description="Stage 0 declared operation",
body=ErrorBody)),
security(("credential"=[])))]
pub async fn find_admin_operation(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    axum::Extension(start): axum::Extension<crate::middleware::RequestStart>,
    ApiQuery(query): ApiQuery<OperationQuery>,
    Bearer(proof): Bearer,
) -> Result<axum::response::Response, ApiError> {
    inspect(
        state,
        start,
        trace,
        proof,
        None,
        ReceiptLookup::Key(query.idempotency_key),
    )
    .await
}
async fn inspect(
    state: crate::app::AppState,
    start: crate::middleware::RequestStart,
    trace: crate::middleware::TraceId,
    proof: anchorfs_core::CredentialProof,
    workspace: Option<anchorfs_core::WorkspaceId>,
    lookup: ReceiptLookup,
) -> Result<axum::response::Response, ApiError> {
    use anchorfs_persistence::{
        authority::{self, CredentialKind, LockMode},
        receipts::{self, ReceiptFilter},
    };
    if matches!(&lookup,ReceiptLookup::Key(key) if key.is_empty() || key.len()>256 || !key.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(ApiError::new(
            anchorfs_core::ErrorCode::InvalidInput,
            trace.0,
        ));
    }
    let result = crate::control::run(
        &state,
        start,
        trace.clone(),
        crate::control::Kind::Read,
        move |tx, _| {
            Box::pin(async move {
                let exists = match workspace {
                    Some(w) => authority::lock_workspace(tx, w).await?.is_some(),
                    None => true,
                };
                let actor = authority::load_actor(tx, &proof, LockMode::Share).await?;
                if !exists || workspace.is_some_and(|w| !authority::same_workspace(&actor, w)) {
                    return Err(authority::AuthorityError::NotFound.into());
                }
                let filter = ReceiptFilter {
                    workspace,
                    credential: actor.id,
                    allow_workspace_actors: workspace.is_some()
                        && actor.kind == CredentialKind::Host,
                };
                let view = receipts::read(tx, filter, lookup)
                    .await?
                    .ok_or(authority::AuthorityError::NotFound)?;
                authority::check_publication_time(tx, &actor, None).await?;
                let followups =
                    crate::receipt::expiry_followups(workspace, view.tombstoned.clone());
                let mut answer = crate::receipt::Answer::metadata(super::views::operation(view)?)?;
                answer.followups = followups;
                Ok(Ok(answer))
            })
        },
    )
    .await;
    crate::receipt::finish(&state, result, axum::http::StatusCode::OK, &trace.0).await
}
