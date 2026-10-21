use super::validate_path;
use crate::{dto::*, error::ApiError, extract::*};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/bytes", operation_id="readRange",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("offset" = u64, Query),("length" = u32, Query),("read_pin" = Option<Uuid>, Query)),
 responses((status=200,description="Verified bytes from one committed manifest",body=BinaryContent,content_type="application/octet-stream",headers(("X-Anchorfs-Generation"=i64,description="Inode generation at read time for unpinned reads; pinned manifest data generation for pinned reads (may lag metadata-only changes)"))), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "fenced", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody)), security(("credential" = [])))]
pub async fn read_range(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: ByteAuthority,
    ApiPath((workspace, handle)): ApiPath<(Uuid, Uuid)>,
    ApiQuery(query): ApiQuery<ReadQuery>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    if query.length == 0 || query.length as usize > state.config.max_request_bytes {
        return Err(ApiError::new(
            anchorfs_core::ErrorCode::InvalidInput,
            context.trace.0,
        ));
    }
    let req = crate::pipeline::read::ReadRequest {
        workspace: anchorfs_core::WorkspaceId(workspace),
        handle: anchorfs_core::HandleId(handle),
        authority: authority.finish()?,
        query,
    };
    let job_state = state.clone();
    let result = crate::pipeline::run(
        &state,
        context.start,
        context.trace,
        crate::control::Kind::Read,
        move |db, deadline, faults, _| {
            crate::pipeline::read::read(db, deadline, faults, job_state, req)
        },
    )
    .await?;
    Ok((
        [
            ("Content-Type", "application/octet-stream"),
            ("X-Anchorfs-Generation", &result.generation.to_string()),
        ],
        result.bytes,
    )
        .into_response())
}
#[utoipa::path(put, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/bytes", operation_id="writeAt",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("offset" = u64, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed durable write and original append offset",body=WriteResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate, operation_failed", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn write_at(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace, handle)): ApiPath<(Uuid, Uuid)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiQuery(query): ApiQuery<WriteQuery>,
    BinaryBody(bytes): BinaryBody,
) -> Result<axum::response::Response, ApiError> {
    write(
        state,
        context,
        authority,
        workspace,
        handle,
        key,
        crate::pipeline::write::WriteKind::At(query.offset),
        bytes,
    )
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/handles/{handle_id}/append", operation_id="append",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("handle_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed durable write and original append offset",body=WriteResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate, operation_failed", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn append(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace, handle)): ApiPath<(Uuid, Uuid)>,
    IdempotencyKey(key): IdempotencyKey,
    BinaryBody(bytes): BinaryBody,
) -> Result<axum::response::Response, ApiError> {
    write(
        state,
        context,
        authority,
        workspace,
        handle,
        key,
        crate::pipeline::write::WriteKind::Append,
        bytes,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn write(
    state: crate::app::AppState,
    context: RequestContext,
    authority: SessionAuthority,
    workspace: Uuid,
    handle: Uuid,
    key: String,
    kind: crate::pipeline::write::WriteKind,
    bytes: bytes::Bytes,
) -> Result<axum::response::Response, ApiError> {
    use crate::pipeline::{
        self,
        common::Context,
        write::{WriteKind, WriteRequest},
    };
    if bytes.is_empty() {
        return Err(ApiError::new(
            anchorfs_core::ErrorCode::InvalidInput,
            context.trace.0,
        ));
    }
    if bytes.len() > state.config.max_request_bytes {
        return Err(ApiError::new(
            anchorfs_core::ErrorCode::PayloadTooLarge,
            context.trace.0,
        ));
    }
    let (op_type, body) = match kind {
        WriteKind::At(offset) => (
            "write_at",
            serde_json::json!({"offset":offset,"sha256":anchorfs_core::sha256(&bytes).to_string()}),
        ),
        WriteKind::Append => (
            "append",
            serde_json::json!({"sha256":anchorfs_core::sha256(&bytes).to_string()}),
        ),
    };
    let digest = crate::receipt::request_digest(
        op_type,
        &[&workspace.to_string(), &handle.to_string()],
        Some(&body),
    );
    let req = WriteRequest {
        context: Context {
            workspace: anchorfs_core::WorkspaceId(workspace),
            authority,
            key,
            digest,
            op_type,
        },
        handle: anchorfs_core::HandleId(handle),
        kind,
        body: bytes,
    };
    let job_state = state.clone();
    let result = pipeline::run(
        &state,
        context.start,
        context.trace.clone(),
        crate::control::Kind::Mutation,
        move |db, deadline, faults, phase| {
            pipeline::write::write(db, deadline, faults, phase, job_state, req)
        },
    )
    .await;
    crate::receipt::finish(
        &state,
        result.map(Ok),
        axum::http::StatusCode::OK,
        &context.trace.0,
    )
    .await
}
#[utoipa::path(put, path="/v1/workspaces/{workspace_id}/files", operation_id="replaceFile",
 request_body(content = String, content_type = "application/octet-stream"),
 params(("Content-Length" = u64, Header, description="Required exact body length"),("X-Anchorfs-Content-SHA256" = String, Header, description="Required 64 hex SHA-256 of the complete body"),("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = String, Query),("expected_generation" = i64, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed conditional whole-file replacement",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate, operation_failed", body = ErrorBody),(status = 410, description = "stale_handle", body = ErrorBody),(status = 413, description = "payload_too_large or file_too_large", body = ErrorBody)), security(("credential" = [])))]
pub async fn replace_file(
    axum::extract::State(state): axum::extract::State<crate::app::AppState>,
    context: RequestContext,
    authority: ByteAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiQuery(query): ApiQuery<ReplaceQuery>,
    content: BinaryStream,
) -> Result<axum::response::Response, ApiError> {
    validate_path(&query.path, &context.trace.0)?;
    if content.length > state.config.max_replace_bytes as u64 {
        return Err(ApiError::new(
            anchorfs_core::ErrorCode::PayloadTooLarge,
            context.trace.0,
        ));
    }
    let authority = authority.finish()?;
    let body = serde_json::json!({"path":query.path,"expected_generation":query.expected_generation,"sha256":content.sha256.to_string(),"length":content.length});
    let digest =
        crate::receipt::request_digest("replace_file", &[&workspace.to_string()], Some(&body));
    let request = crate::pipeline::replace::ReplaceRequest {
        context: crate::pipeline::common::Context {
            workspace: anchorfs_core::WorkspaceId(workspace),
            authority,
            key,
            digest,
            op_type: "replace_file",
        },
        path: anchorfs_core::WorkspacePath::parse(query.path).expect("validated path"),
        expected_generation: query.expected_generation,
        content,
    };
    let job_state = state.clone();
    let result = crate::pipeline::run(
        &state,
        context.start,
        context.trace.clone(),
        crate::control::Kind::Mutation,
        move |db, deadline, faults, phase| {
            crate::pipeline::replace::replace(db, deadline, faults, phase, job_state, request)
        },
    )
    .await;
    crate::receipt::finish(
        &state,
        result.map(Ok),
        axum::http::StatusCode::OK,
        &context.trace.0,
    )
    .await
}
