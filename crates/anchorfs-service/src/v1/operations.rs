use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/operations/{operation_id}", operation_id="getOperation",

 params(("workspace_id" = Uuid, Path),("operation_id" = Uuid, Path)),
 responses((status=200,description="Future success contract; Stage 0 returns 501",body=OperationStatus), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn get_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid, Uuid)>,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/operations", operation_id="findOperation",

 params(("workspace_id" = Uuid, Path),("idempotency_key" = String, Query)),
 responses((status=200,description="Future success contract; Stage 0 returns 501",body=OperationStatus), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn find_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<OperationQuery>,
) -> Result<(), ApiError> {
    let _ = query;
    Err(ApiError::not_implemented(trace.0))
}

#[utoipa::path(get,path="/v1/admin-operations/{operation_id}",operation_id="getAdminOperation",params(("operation_id"=Uuid,Path)),responses((status=200,description="Future credential-scoped receipt, including expired tombstone",body=OperationStatus),(status=400,description="invalid_input",body=ErrorBody),(status=401,description="unauthenticated",body=ErrorBody),(status=403,description="forbidden",body=ErrorBody),(status=404,description="not_found",body=ErrorBody),(status=501,description="Stage 0 declared operation",body=ErrorBody)),security(("credential"=[])))]
pub async fn get_admin_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiPath(_ids): ApiPath<(Uuid,)>,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}

#[utoipa::path(get,path="/v1/admin-operations",operation_id="findAdminOperation",params(("idempotency_key"=String,Query)),responses((status=200,description="Future credential-scoped receipt, including expired tombstone",body=OperationStatus),(status=400,description="invalid_input",body=ErrorBody),(status=401,description="unauthenticated",body=ErrorBody),(status=403,description="forbidden",body=ErrorBody),(status=404,description="not_found",body=ErrorBody),(status=501,description="Stage 0 declared operation",body=ErrorBody)),security(("credential"=[])))]
pub async fn find_admin_operation(
    axum::Extension(trace): axum::Extension<crate::middleware::TraceId>,
    ApiQuery(_query): ApiQuery<OperationQuery>,
) -> Result<(), ApiError> {
    Err(ApiError::not_implemented(trace.0))
}
