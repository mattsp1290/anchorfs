use super::validate_path;
use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/read-pins", operation_id="createReadPin",
 request_body = PathBody,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_read_pin(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<PathBody>,
) -> Result<(), ApiError> {
    validate_path(&body.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/read-pins/{read_pin_id}", operation_id="releaseReadPin",

 params(("workspace_id" = Uuid, Path),("read_pin_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn release_read_pin(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    _key: IdempotencyKey,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
