use super::validate_path;
use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/bytes", operation_id="readRange",

 params(("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("offset" = u64, Query),("length" = u32, Query),("read_pin" = Option<Uuid>, Query)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn read_range(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    ApiQuery(query): ApiQuery<ReadQuery>,
) -> Result<(), ApiError> {
    let _ = query;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(put, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/bytes", operation_id="writeAt",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("offset" = u64, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn write_at(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    _key: IdempotencyKey,
    ApiQuery(query): ApiQuery<WriteQuery>,
    _body: BinaryBody,
) -> Result<(), ApiError> {
    let _ = query;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/append", operation_id="append",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn append(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
    _key: IdempotencyKey,
    _body: BinaryBody,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(put, path="/v1/workspaces/{workspace_id}/files", operation_id="replaceFile",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("workspace_id" = Uuid, Path),("path" = String, Query),("expected_generation" = i64, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn replace_file(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiQuery(query): ApiQuery<ReplaceQuery>,
    _body: BinaryBody,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
