use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces", operation_id="createWorkspace",
 request_body = CreateWorkspace,
 params(("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_workspace(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> Result<(), ApiError> {
    let _ = body;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}", operation_id="getWorkspace",

 params(("workspace_id" = Uuid, Path)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn get_workspace(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
