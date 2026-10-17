use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(post, path="/v1/credentials", operation_id="createCredential",
 request_body = CreateCredential,
 params(("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_credential(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    _key: IdempotencyKey,
    ApiJson(body): ApiJson<CreateCredential>,
) -> Result<(), ApiError> {
    let _ = body;
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(post, path="/v1/credentials/{credential_id}/rotate", operation_id="rotateCredential",

 params(("credential_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn rotate_credential(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(delete, path="/v1/credentials/{credential_id}", operation_id="revokeCredential",

 params(("credential_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn revoke_credential(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    _key: IdempotencyKey,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
