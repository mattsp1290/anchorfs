use super::validate_path;
use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/stat", operation_id="statPath",

 params(("workspace_id" = Uuid, Path),("path" = String, Query)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn stat_path(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/directory", operation_id="listDirectory",

 params(("workspace_id" = Uuid, Path),("path" = String, Query),("cursor" = Option<String>, Query),("limit" = Option<u32>, Query)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn list_directory(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/directories", operation_id="createDirectory",
 request_body = CreateDirectory,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_directory(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<CreateDirectory>,
) -> Result<(), ApiError> {
    validate_path(&body.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/directories", operation_id="removeDirectory",

 params(("workspace_id" = Uuid, Path),("path" = String, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn remove_directory(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/files", operation_id="unlink",

 params(("workspace_id" = Uuid, Path),("path" = String, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn unlink(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/links", operation_id="createLink",
 request_body = CreateLink,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_link(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<CreateLink>,
) -> Result<(), ApiError> {
    validate_path(&body.existing_path, &trace.0)?;
    validate_path(&body.new_path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/symlinks", operation_id="createSymlink",
 request_body = CreateSymlink,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_symlink(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<CreateSymlink>,
) -> Result<(), ApiError> {
    validate_path(&body.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/symlinks", operation_id="readSymlink",

 params(("workspace_id" = Uuid, Path),("path" = String, Query)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn read_symlink(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<(), ApiError> {
    validate_path(&query.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/rename", operation_id="rename",
 request_body = Rename,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn rename(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<Rename>,
) -> Result<(), ApiError> {
    validate_path(&body.from, &trace.0)?;
    validate_path(&body.to, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(patch, path="/v1/workspaces/{workspace_id}/attributes", operation_id="setAttributes",
 request_body = SetAttributes,
 params(("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn set_attributes(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<SetAttributes>,
) -> Result<(), ApiError> {
    validate_path(&body.path, &trace.0)?;
    Err(ApiError::not_implemented(trace.0))
}
