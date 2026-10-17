use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/operations/{operation_id}", operation_id="getOperation",

 params(("workspace_id" = Uuid, Path),("operation_id" = Uuid, Path)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn get_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/operations", operation_id="findOperation",

 params(("workspace_id" = Uuid, Path),("idempotency_key" = String, Query)),
 responses((status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn find_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<OperationQuery>,
) -> Result<(), ApiError> {
    let _ = query;
    Err(ApiError::not_implemented(trace.0))
}
