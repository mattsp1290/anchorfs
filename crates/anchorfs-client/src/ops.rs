use crate::{
    Client,
    error::{ApiFailure, ClientError},
    retry::{IdempotencyKey, Resolver, execute},
    wire::*,
};
use anchorfs_core::ErrorCode;
use bytes::Bytes;
use reqwest::{Method, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::time::Duration;

use uuid::Uuid;

pub(crate) type Response = (Bytes, HeaderMap);
#[derive(Clone)]
pub(crate) enum Body {
    Empty,
    Json(Value),
    Bytes(Bytes),
}
#[derive(Clone, Copy)]
pub(crate) enum Deadline {
    Metadata,
    Data,
}
#[derive(Clone, Copy)]
enum Accept {
    Json,
    Octets,
}
impl Accept {
    fn header(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Octets => "application/octet-stream",
        }
    }
}
pub(crate) struct Route {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Body,
    pub deadline: Deadline,
    pub resolver: Resolver,
    accept: Accept,
}
impl Route {
    fn new(method: Method, path: String, body: Body) -> Self {
        Self {
            method,
            path,
            body,
            query: vec![],
            deadline: Deadline::Metadata,
            resolver: Resolver::Workspace,
            accept: Accept::Json,
        }
    }
    fn query(mut self, query: Vec<(String, String)>) -> Self {
        self.query = query;
        self
    }
    fn data(mut self) -> Self {
        self.deadline = Deadline::Data;
        self
    }
    fn octets(mut self) -> Self {
        self.accept = Accept::Octets;
        self
    }
    fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = resolver;
        self
    }
    pub fn timeout(&self, client: &Client) -> Duration {
        match self.deadline {
            Deadline::Metadata => client.metadata_deadline,
            Deadline::Data => client.data_deadline,
        }
    }
}
impl Client {
    fn workspace_route(&self, suffix: &str) -> String {
        format!("/v1/workspaces/{}{suffix}", self.workspace)
    }
    #[tracing::instrument(skip_all, fields(trace_id = tracing::field::Empty))]
    pub(crate) async fn once(
        &self,
        route: &Route,
        key: Option<&IdempotencyKey>,
        timeout: Duration,
    ) -> Result<Response, ClientError> {
        let Route {
            method,
            path,
            query,
            body,
            ..
        } = route;
        let mut request = self
            .authority
            .apply(self.http.request(method.clone(), self.endpoint.route(path)))
            .query(query)
            .header("Accept", route.accept.header())
            .timeout(timeout);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key.as_str());
        }
        request = match body {
            Body::Empty => request,
            Body::Json(value) => request.json(value),
            Body::Bytes(value) => request
                .header("Content-Type", "application/octet-stream")
                .body(value.clone()),
        };
        let response = request.send().await.map_err(ClientError::transport)?;
        let status = response.status();
        let headers = response.headers().clone();
        let trace_id = headers
            .get("X-Anchorfs-Trace-Id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        tracing::Span::current().record("trace_id", trace_id.as_str());
        tracing::debug!(%trace_id, status = status.as_u16(), "native API response");
        let bytes = response.bytes().await.map_err(ClientError::transport)?;
        if status.is_success() {
            return Ok((bytes, headers));
        }
        let mut error = serde_json::from_slice::<ApiFailure>(&bytes).unwrap_or(ApiFailure {
            status: status.as_u16(),
            code: "unknown".into(),
            message: "unrecognized API error".into(),
            trace_id: trace_id.clone(),
            operation_id: None,
            retry_after_ms: None,
        });
        error.status = status.as_u16();
        error.trace_id = trace_id;
        Err(ClientError::Api(error))
    }
    async fn decoded<T: DeserializeOwned>(&self, route: Route) -> Result<T, ClientError> {
        decode(execute(self, &route, None).await?.0)
    }
    async fn get<T: DeserializeOwned>(
        &self,
        path: String,
        query: Vec<(String, String)>,
    ) -> Result<T, ClientError> {
        self.decoded(Route::new(Method::GET, path, Body::Empty).query(query))
            .await
    }
    async fn mutation<T: DeserializeOwned>(
        &self,
        route: Route,
        key: &IdempotencyKey,
    ) -> Result<T, ClientError> {
        match execute(self, &route, Some(key)).await {
            Ok((bytes, _)) => match decode(bytes) {
                Ok(value) => Ok(value),
                Err(_) => self.resolve_by_key(key, route.resolver).await,
            },
            Err(ClientError::Ambiguous(_)) => self.resolve_by_key(key, route.resolver).await,
            Err(error) => Err(error),
        }
    }
    pub async fn resolve_by_key<T: DeserializeOwned>(
        &self,
        key: &IdempotencyKey,
        resolver: Resolver,
    ) -> Result<T, ClientError> {
        let status = match resolver {
            Resolver::Workspace => self.find_operation(key).await,
            Resolver::Admin => self.find_admin_operation(key).await,
        };
        let status = match status {
            Ok(status) => status,
            Err(ClientError::Api(e)) if e.status == 404 => {
                return Err(resolution_error(ErrorCode::OperationIndeterminate, None));
            }
            Err(_) => return Err(resolution_error(ErrorCode::OutcomeUnknown, None)),
        };
        match status.state {
            OperationState::Committed => serde_json::from_value(
                status
                    .result
                    .ok_or_else(|| ClientError::Decode("committed receipt has no result".into()))?,
            )
            .map_err(|e| ClientError::Decode(e.to_string())),
            OperationState::Failed => Err(resolution_error(
                ErrorCode::OperationFailed,
                Some(status.operation_id),
            )),
            OperationState::Pending => Err(resolution_error(
                ErrorCode::OutcomeUnknown,
                Some(status.operation_id),
            )),
            OperationState::Indeterminate => Err(resolution_error(
                ErrorCode::OperationIndeterminate,
                Some(status.operation_id),
            )),
        }
    }
    pub async fn get_workspace(&self) -> Result<WorkspaceInfo, ClientError> {
        self.get(self.workspace_route(""), vec![]).await
    }
    pub async fn stat(&self, path: Option<&str>) -> Result<InodeAttributes, ClientError> {
        self.get(self.workspace_route("/stat"), path_query(path))
            .await
    }
    pub async fn list_directory(
        &self,
        path: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<DirectoryPage, ClientError> {
        let mut q = path_query(path);
        if let Some(c) = cursor {
            q.push(("cursor".into(), c.into()));
        }
        q.push(("limit".into(), limit.to_string()));
        self.get(self.workspace_route("/directory"), q).await
    }
    pub async fn read_symlink(&self, path: &str) -> Result<SymlinkContents, ClientError> {
        self.get(self.workspace_route("/symlinks"), path_query(Some(path)))
            .await
    }
    pub async fn find_operation(
        &self,
        key: &IdempotencyKey,
    ) -> Result<OperationStatus, ClientError> {
        self.get(
            self.workspace_route("/operations"),
            vec![("idempotency_key".into(), key.as_str().into())],
        )
        .await
    }
    pub async fn get_operation(&self, id: Uuid) -> Result<OperationStatus, ClientError> {
        self.get(self.workspace_route(&format!("/operations/{id}")), vec![])
            .await
    }
    pub async fn find_admin_operation(
        &self,
        key: &IdempotencyKey,
    ) -> Result<OperationStatus, ClientError> {
        self.get(
            "/v1/admin-operations".into(),
            vec![("idempotency_key".into(), key.as_str().into())],
        )
        .await
    }
    pub async fn get_admin_operation(&self, id: Uuid) -> Result<OperationStatus, ClientError> {
        self.get(format!("/v1/admin-operations/{id}"), vec![]).await
    }
    pub async fn create_session(
        &self,
        credential_id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<SessionGrant, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/sessions"),
                Body::Json(json!({"credential_id": credential_id, "bind_to_workflow_lease": null})),
            )
            .resolver(Resolver::Admin),
            key,
        )
        .await
    }
    pub async fn renew_session(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<SessionGrant, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route(&format!("/sessions/{id}/renew")),
                Body::Empty,
            ),
            key,
        )
        .await
    }
    pub async fn revoke_session(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::DELETE,
                self.workspace_route(&format!("/sessions/{id}")),
                Body::Empty,
            ),
            key,
        )
        .await
    }
    pub async fn open_handle(
        &self,
        body: &OpenHandle,
        key: &IdempotencyKey,
    ) -> Result<HandleGrant, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/handles"),
                Body::Json(json!(body)),
            ),
            key,
        )
        .await
    }
    pub async fn renew_handle(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<HandleGrant, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route(&format!("/handles/{id}/renew")),
                Body::Empty,
            ),
            key,
        )
        .await
    }
    pub async fn close_handle(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::DELETE,
                self.workspace_route(&format!("/handles/{id}")),
                Body::Empty,
            ),
            key,
        )
        .await
    }
    pub async fn truncate(
        &self,
        id: Uuid,
        size: u64,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route(&format!("/handles/{id}/truncate")),
                Body::Json(json!({"size":size})),
            )
            .data(),
            key,
        )
        .await
    }
    pub async fn fsync(&self, id: Uuid) -> Result<InodeAttributes, ClientError> {
        self.decoded(
            Route::new(
                Method::POST,
                self.workspace_route(&format!("/handles/{id}/fsync")),
                Body::Empty,
            )
            .data(),
        )
        .await
    }
    pub async fn read_range(
        &self,
        id: Uuid,
        offset: u64,
        length: u32,
    ) -> Result<(Bytes, i64), ClientError> {
        let route = Route::new(
            Method::GET,
            self.workspace_route(&format!("/handles/{id}/bytes")),
            Body::Empty,
        )
        .query(vec![
            ("offset".into(), offset.to_string()),
            ("length".into(), length.to_string()),
        ])
        .data()
        .octets();
        let (bytes, headers) = execute(self, &route, None).await?;
        let generation = headers
            .get("X-Anchorfs-Generation")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ClientError::Decode("missing or invalid generation".into()))?;
        Ok((bytes, generation))
    }
    pub async fn write_at(
        &self,
        id: Uuid,
        offset: u64,
        bytes: Bytes,
        key: &IdempotencyKey,
    ) -> Result<WriteResult, ClientError> {
        validate_bytes(&bytes)?;
        self.mutation(
            Route::new(
                Method::PUT,
                self.workspace_route(&format!("/handles/{id}/bytes")),
                Body::Bytes(bytes),
            )
            .query(vec![("offset".into(), offset.to_string())])
            .data(),
            key,
        )
        .await
    }
    pub async fn append(
        &self,
        id: Uuid,
        bytes: Bytes,
        key: &IdempotencyKey,
    ) -> Result<WriteResult, ClientError> {
        validate_bytes(&bytes)?;
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route(&format!("/handles/{id}/append")),
                Body::Bytes(bytes),
            )
            .data(),
            key,
        )
        .await
    }
    pub async fn create_directory(
        &self,
        path: &str,
        mode: u32,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/directories"),
                Body::Json(json!({"path":path,"mode":mode})),
            ),
            key,
        )
        .await
    }
    pub async fn remove_directory(
        &self,
        path: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::DELETE,
                self.workspace_route("/directories"),
                Body::Empty,
            )
            .query(path_query(Some(path))),
            key,
        )
        .await
    }
    pub async fn unlink(
        &self,
        path: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(Method::DELETE, self.workspace_route("/files"), Body::Empty)
                .query(path_query(Some(path))),
            key,
        )
        .await
    }
    pub async fn create_link(
        &self,
        existing_path: &str,
        new_path: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/links"),
                Body::Json(json!({"existing_path":existing_path,"new_path":new_path})),
            ),
            key,
        )
        .await
    }
    pub async fn create_symlink(
        &self,
        path: &str,
        target: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/symlinks"),
                Body::Json(json!({"path":path,"target":target})),
            ),
            key,
        )
        .await
    }
    pub async fn rename(
        &self,
        from: &str,
        to: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::POST,
                self.workspace_route("/rename"),
                Body::Json(json!({"from":from,"to":to})),
            ),
            key,
        )
        .await
    }
    pub async fn set_attributes(
        &self,
        attributes: &SetAttributes,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Route::new(
                Method::PATCH,
                self.workspace_route("/attributes"),
                Body::Json(
                    serde_json::to_value(attributes).map_err(|_| {
                        ClientError::InvalidInput("invalid explicit timestamp".into())
                    })?,
                ),
            ),
            key,
        )
        .await
    }
}
fn decode<T: DeserializeOwned>(bytes: Bytes) -> Result<T, ClientError> {
    serde_json::from_slice(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
}
fn path_query(path: Option<&str>) -> Vec<(String, String)> {
    path.map(|p| vec![("path".into(), p.into())])
        .unwrap_or_default()
}
fn validate_bytes(bytes: &Bytes) -> Result<(), ClientError> {
    if bytes.is_empty() || bytes.len() > u32::MAX as usize {
        Err(ClientError::InvalidInput(
            "byte body must contain 1..=u32::MAX bytes".into(),
        ))
    } else {
        Ok(())
    }
}
fn resolution_error(code: ErrorCode, operation_id: Option<Uuid>) -> ClientError {
    ClientError::Api(ApiFailure {
        status: 409,
        code: code.into(),
        message: "receipt resolution did not establish success".into(),
        trace_id: String::new(),
        operation_id,
        retry_after_ms: None,
    })
}
