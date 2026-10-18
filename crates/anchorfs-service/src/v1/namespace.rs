use super::validate_path;
use crate::{
    app::AppState,
    control,
    receipt::{self, Authority, Published},
};
use crate::{dto::*, error::ApiError, extract::*};
use anchorfs_core::{WorkspaceId, WorkspacePath};
use anchorfs_persistence::{
    authority::LeaseMode,
    namespace,
    receipts::{ReceiptRequest, ReceiptScope},
};
use axum::{extract::State, http::StatusCode, response::Response};
use uuid::Uuid;
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/stat", operation_id="statPath",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = Option<String>, Query, description="Omit for workspace root; supplied paths must be nonempty and relative")),
 responses((status=200,description="Committed namespace metadata",body=InodeAttributes), (status=409,description="not_directory or fenced",body=ErrorBody), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn stat_path(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<LookupQuery>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    if let Some(path) = &query.path {
        validate_path(path, &context.trace.0)?;
    }
    read(
        state,
        context,
        authority,
        WorkspaceId(workspace),
        Read::Stat(query),
    )
    .await
    .map(axum::Json)
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/directory", operation_id="listDirectory",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = Option<String>, Query, description="Omit for workspace root; supplied paths must be nonempty and relative"),("cursor" = Option<String>, Query),("limit" = Option<u32>, Query)),
 responses((status=200,description="Committed namespace metadata",body=DirectoryPage), (status=409,description="not_directory or fenced",body=ErrorBody), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn list_directory(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    if let Some(path) = &query.path {
        validate_path(path, &context.trace.0)?;
    }
    read(
        state,
        context,
        authority,
        WorkspaceId(workspace),
        Read::List(query),
    )
    .await
    .map(axum::Json)
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/directories", operation_id="createDirectory",
 request_body = CreateDirectory,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_directory(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateDirectory>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&body.path, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Mkdir(body),
    )
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/directories", operation_id="removeDirectory",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = String, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn remove_directory(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&query.path, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Rmdir(query.path),
    )
    .await
}
#[utoipa::path(delete, path="/v1/workspaces/{workspace_id}/files", operation_id="unlink",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = String, Query),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn unlink(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&query.path, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Unlink(query.path),
    )
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/links", operation_id="createLink",
 request_body = CreateLink,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_link(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateLink>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&body.existing_path, &trace.0)?;
    validate_path(&body.new_path, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Link(body),
    )
    .await
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/symlinks", operation_id="createSymlink",
 request_body = CreateSymlink,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn create_symlink(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<CreateSymlink>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&body.path, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Symlink(body),
    )
    .await
}
#[utoipa::path(get, path="/v1/workspaces/{workspace_id}/symlinks", operation_id="readSymlink",

 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("path" = String, Query)),
 responses((status=200,description="Committed namespace metadata",body=SymlinkContents), (status=409,description="not_directory or fenced",body=ErrorBody), (status=404,description="not_found",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody)), security(("credential" = [])))]
pub async fn read_symlink(
    State(state): State<AppState>,
    context: RequestContext,
    authority: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<PathQuery>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    validate_path(&query.path, &context.trace.0)?;
    read(
        state,
        context,
        authority,
        WorkspaceId(workspace),
        Read::Symlink(query),
    )
    .await
    .map(axum::Json)
}
#[utoipa::path(post, path="/v1/workspaces/{workspace_id}/rename", operation_id="rename",
 request_body = Rename,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn rename(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<Rename>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    validate_path(&body.from, &trace.0)?;
    validate_path(&body.to, &trace.0)?;
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Rename(body),
    )
    .await
}
#[utoipa::path(patch, path="/v1/workspaces/{workspace_id}/attributes", operation_id="setAttributes",
 request_body = SetAttributes,
 params(("X-Anchorfs-Session-Id" = Uuid, Header), ("X-Anchorfs-Session-Epoch" = i64, Header, minimum=1),("X-Anchorfs-Workflow-Token" = Option<String>, Header, description="64 hex characters; required with active workflow authority"),("X-Anchorfs-Workflow-Epoch" = Option<i64>, Header, minimum=1),("workspace_id" = Uuid, Path),("Idempotency-Key" = String, Header, description = "Required retry key, 1–256 visible ASCII bytes")),
 responses((status=200,description="Committed namespace mutation",body=MutationResult), (status=404,description="not_found",body=ErrorBody),(status=507,description="quota_exceeded or capacity_exhausted",body=ErrorBody),(status = 501, description = "Stage 0 declared operation", body = ErrorBody),(status = 400, description = "invalid_input", body = ErrorBody),(status = 401, description = "unauthenticated", body = ErrorBody),(status = 403, description = "forbidden", body = ErrorBody),(status = 503, description = "dependency_unavailable; outcome_unknown after mutation begins (Stage 1)", body = ErrorBody),(status = 409, description = "generation_conflict, idempotency_mismatch, workspace_busy, fenced, operation_indeterminate", body = ErrorBody)), security(("credential" = [])))]
pub async fn set_attributes(
    State(state): State<AppState>,
    context: RequestContext,
    SessionAuthority {
        binding,
        workflow,
        proof,
    }: SessionAuthority,
    ApiPath((workspace,)): ApiPath<(Uuid,)>,
    IdempotencyKey(key): IdempotencyKey,
    ApiJson(body): ApiJson<SetAttributes>,
) -> Result<Response, ApiError> {
    let trace = &context.trace;
    if let Some(path) = &body.path {
        validate_path(path, &trace.0)?;
    }
    mutate(
        MutationCall {
            state,
            context,
            binding,
            workspace: WorkspaceId(workspace),
            key,
            workflow,
            proof,
        },
        Mutation::Attributes(body),
    )
    .await
}

struct MutationCall {
    state: AppState,
    context: RequestContext,
    binding: SessionBinding,
    workspace: WorkspaceId,
    key: String,
    workflow: Option<anchorfs_persistence::authority::WorkflowProof>,
    proof: anchorfs_core::CredentialProof,
}
enum Mutation {
    Mkdir(CreateDirectory),
    Rmdir(String),
    Unlink(String),
    Link(CreateLink),
    Symlink(CreateSymlink),
    Rename(Rename),
    Attributes(SetAttributes),
}
impl Mutation {
    fn request(&self) -> (&'static str, serde_json::Value) {
        use serde_json::{json, to_value};
        match self {
            Self::Mkdir(b) => ("directory_create", to_value(b).expect("request JSON")),
            Self::Rmdir(path) => ("directory_remove", json!({"path":path})),
            Self::Unlink(path) => ("unlink", json!({"path":path})),
            Self::Link(b) => ("link_create", to_value(b).expect("request JSON")),
            Self::Symlink(b) => ("symlink_create", to_value(b).expect("request JSON")),
            Self::Rename(b) => ("rename", to_value(b).expect("request JSON")),
            Self::Attributes(b) => ("attributes_set", to_value(b).expect("request JSON")),
        }
    }
    async fn apply(
        self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        guard: &namespace::NamespaceGuard,
        caller: &namespace::Caller,
    ) -> Result<namespace::Mutated, ApiError> {
        let path = |s: &str| WorkspacePath::parse(s).expect("validated path");
        Ok(match self {
            Self::Mkdir(b) => namespace::mkdir(tx, guard, caller, &path(&b.path), b.mode).await?,
            Self::Rmdir(p) => namespace::rmdir(tx, guard, caller, &path(&p)).await?,
            Self::Unlink(p) => namespace::unlink(tx, guard, caller, &path(&p)).await?,
            Self::Link(b) => {
                namespace::link(
                    tx,
                    guard,
                    caller,
                    &path(&b.existing_path),
                    &path(&b.new_path),
                )
                .await?
            }
            Self::Symlink(b) => {
                namespace::symlink(tx, guard, caller, &path(&b.path), &b.target).await?
            }
            Self::Rename(b) => {
                namespace::rename(tx, guard, caller, &path(&b.from), &path(&b.to)).await?
            }
            Self::Attributes(b) => {
                namespace::set_attributes(
                    tx,
                    guard,
                    caller,
                    b.path.as_deref().map(path).as_ref(),
                    b.mode,
                    b.uid,
                    b.gid,
                )
                .await?
            }
        })
    }
}
async fn mutate(call: MutationCall, mutation: Mutation) -> Result<Response, ApiError> {
    let MutationCall {
        state,
        context,
        binding,
        workspace,
        key,
        workflow,
        proof,
    } = call;
    let (op_type, body) = mutation.request();
    let digest = receipt::request_digest(op_type, &[&workspace.to_string()], Some(&body));
    let retention = state.config.receipt_retention;
    let result = control::run(
        &state,
        context.start,
        context.trace.clone(),
        control::Kind::Mutation,
        move |tx, budget| {
            Box::pin(async move {
                let p = super::prologue(
                    tx,
                    &proof,
                    workspace,
                    &binding,
                    workflow.as_ref(),
                    super::Gate {
                        role_mutation: true,
                        lease: LeaseMode::Mutation,
                    },
                )
                .await?;
                let scope = ReceiptScope::Workspace {
                    workspace,
                    credential: p.actor.id,
                    session: p.session.id,
                    session_epoch: p.session.session_epoch,
                    authorization_epoch: p.workspace.authorization_epoch,
                    mutation_epoch: p.workspace.mutation_epoch,
                };
                receipt::execute(
                    tx,
                    Authority {
                        workflow: Some(receipt::WorkflowAuthority {
                            workspace: &p.workspace,
                            proof: workflow.as_ref(),
                            mode: LeaseMode::Mutation,
                        }),
                        actor: &p.actor,
                        session: Some(&p.session),
                    },
                    scope,
                    ReceiptRequest {
                        key: &key,
                        op_type,
                        digest,
                        credential_epoch: p.actor.epoch,
                        retention,
                        wait_bound: budget.remaining(),
                    },
                    move |tx, operation| {
                        Box::pin(async move {
                            let guard = namespace::lock(tx, workspace).await?;
                            let m = mutation.apply(tx, &guard, &p.caller).await?;
                            Published::metadata(MutationResult {
                                operation_id: operation.0,
                                inode_id: Some(m.inode_id),
                                generation: Some(m.generation),
                            })
                        })
                    },
                )
                .await
            })
        },
    )
    .await;
    receipt::response(result, StatusCode::OK, &context.trace.0)
}

enum Read {
    Stat(LookupQuery),
    List(ListQuery),
    Symlink(PathQuery),
}
async fn read(
    state: AppState,
    context: RequestContext,
    authority: SessionAuthority,
    workspace: WorkspaceId,
    request: Read,
) -> Result<serde_json::Value, ApiError> {
    let key = state.cursor_key;
    let max_page = state.config.max_list_page;
    let ttl = state.config.cursor_ttl;
    control::run(
        &state,
        context.start,
        context.trace,
        control::Kind::Read,
        move |tx, _| {
            Box::pin(async move {
                let p = super::prologue(
                    tx,
                    &authority.proof,
                    workspace,
                    &authority.binding,
                    authority.workflow.as_ref(),
                    super::Gate {
                        role_mutation: false,
                        lease: LeaseMode::Read,
                    },
                )
                .await?;
                let path = match &request {
                    Read::Stat(q) => q.path.as_deref(),
                    Read::List(q) => q.path.as_deref(),
                    Read::Symlink(q) => Some(q.path.as_str()),
                }
                .map(|s| WorkspacePath::parse(s).expect("validated path"));
                let row = namespace::resolve(tx, workspace, path.as_ref(), &p.caller).await?;
                let value = match request {
                    Read::Stat(_) => serde_json::to_value(super::views::attributes(&row)?),
                    Read::Symlink(_) => {
                        if row.kind != "symlink" {
                            return Err(namespace::NamespaceError::InvalidInput.into());
                        }
                        serde_json::to_value(SymlinkContents {
                            target: row.symlink_target.expect("symlink constraint"),
                            generation: row.generation,
                        })
                    }
                    Read::List(q) => {
                        if row.kind != "directory" {
                            return Err(namespace::NamespaceError::NotDirectory.into());
                        }
                        namespace::access(
                            &row,
                            &p.caller,
                            anchorfs_core::Access {
                                read: true,
                                exec: true,
                                ..Default::default()
                            },
                        )?;
                        let limit = q.limit.unwrap_or(max_page).min(max_page);
                        if limit == 0 {
                            return Err(namespace::NamespaceError::InvalidInput.into());
                        }
                        let now: i64 = sqlx::query_scalar(
                            "SELECT (extract(epoch FROM clock_timestamp())*1000)::bigint",
                        )
                        .fetch_one(&mut **tx)
                        .await?;
                        let after = q
                            .cursor
                            .as_deref()
                            .map(|text| crate::cursor::decode(text, &key, now, workspace.0, row.id))
                            .transpose()?
                            .map(|c| c.last_name);
                        let rows = namespace::list(
                            tx,
                            workspace,
                            row.id,
                            after.as_deref(),
                            i64::from(limit) + 1,
                        )
                        .await?;
                        let generation = namespace::current_generation(tx, workspace).await?;
                        let more = rows.len() > limit as usize;
                        let entries = rows
                            .into_iter()
                            .take(limit as usize)
                            .map(|(name, inode)| {
                                Ok(DirectoryEntry {
                                    name,
                                    attributes: super::views::attributes(&inode)?,
                                })
                            })
                            .collect::<Result<Vec<_>, ApiError>>()?;
                        let next_cursor = if more {
                            Some(crate::cursor::encode(
                                &crate::cursor::ListCursor {
                                    workspace: workspace.0,
                                    parent_inode: row.id,
                                    last_name: entries.last().expect("nonzero limit").name.clone(),
                                    namespace_generation: generation,
                                    expires_unix_ms: now.saturating_add(
                                        ttl.as_millis().min(i64::MAX as u128) as i64,
                                    ),
                                },
                                &key,
                            ))
                        } else {
                            None
                        };
                        serde_json::to_value(DirectoryPage {
                            entries,
                            next_cursor,
                            namespace_generation: generation,
                        })
                    }
                };
                value
                    .map_err(|_| ApiError::new(anchorfs_core::ErrorCode::DependencyUnavailable, ""))
            })
        },
    )
    .await
}
