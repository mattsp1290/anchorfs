use crate::{
    Client,
    error::{ApiFailure, ClientError},
    retry::{Budget, IdempotencyKey, Resolver},
    wire::*,
};
use bytes::Bytes;
use reqwest::{Method, header::HeaderMap};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use uuid::Uuid;

type Response = (Bytes, HeaderMap);
#[derive(Clone)]
enum Body {
    Empty,
    Json(Value),
    Bytes(Bytes),
}
impl Client {
    fn workspace_route(&self, suffix: &str) -> String {
        format!("/v1/workspaces/{}{suffix}", self.workspace)
    }
    #[tracing::instrument(skip_all, fields(trace_id = tracing::field::Empty))]
    async fn once(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: &Body,
        key: Option<&IdempotencyKey>,
        timeout: Duration,
    ) -> Result<Response, ClientError> {
        let accept = if method == Method::GET && path.ends_with("/bytes") {
            "application/octet-stream"
        } else {
            "application/json"
        };
        let mut request = self
            .authority()
            .apply(self.http.request(method, self.endpoint.route(path)))
            .query(query)
            .header("Accept", accept)
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
    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Vec<(String, String)>,
        body: Body,
        key: Option<&IdempotencyKey>,
        deadline: Duration,
    ) -> Result<Response, ClientError> {
        let timeout = deadline.saturating_add(Duration::from_secs(1));
        let budget = Budget::new(timeout.saturating_mul(3));
        let mut last = ClientError::Deadline;
        let mut ambiguous = false;
        for attempt in 0..budget.attempts {
            let remaining = budget.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self
                .once(
                    method.clone(),
                    path,
                    &query,
                    &body,
                    key,
                    timeout.min(remaining),
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(error) => {
                    ambiguous |= matches!(error, ClientError::Transport(_) | ClientError::Deadline)
                        || error.code() == Some("outcome_unknown");
                    let retry = matches!(error, ClientError::Transport(_) | ClientError::Deadline)
                        || error.code() == Some("dependency_unavailable")
                        || (key.is_some() && error.code() == Some("outcome_unknown"));
                    if !retry {
                        return Err(error);
                    }
                    let delay = match &error {
                        ClientError::Api(e) => e.retry_after_ms.unwrap_or(100),
                        _ => 100,
                    };
                    last = error;
                    if attempt + 1 < budget.attempts {
                        tokio::time::sleep(
                            Duration::from_millis(delay)
                                .min(budget.deadline.saturating_duration_since(Instant::now())),
                        )
                        .await;
                    }
                }
            }
        }
        if key.is_some() && ambiguous && last.code() == Some("dependency_unavailable") {
            Err(ClientError::Transport(
                "an earlier attempt had an unresolved response".into(),
            ))
        } else {
            Err(last)
        }
    }
    async fn get<T: DeserializeOwned>(
        &self,
        suffix: &str,
        query: Vec<(String, String)>,
    ) -> Result<T, ClientError> {
        decode(
            self.request(
                Method::GET,
                &self.workspace_route(suffix),
                query,
                Body::Empty,
                None,
                self.metadata_deadline,
            )
            .await?
            .0,
        )
    }
    async fn mutation<T: DeserializeOwned>(
        &self,
        method: Method,
        suffix: &str,
        body: Body,
        query: Vec<(String, String)>,
        key: &IdempotencyKey,
        data: bool,
    ) -> Result<T, ClientError> {
        let deadline = if data {
            self.data_deadline
        } else {
            self.metadata_deadline
        };
        match self
            .request(
                method,
                &self.workspace_route(suffix),
                query,
                body,
                Some(key),
                deadline,
            )
            .await
        {
            Ok((bytes, _)) => decode(bytes),
            Err(error)
                if matches!(error, ClientError::Transport(_) | ClientError::Deadline)
                    || error.code() == Some("outcome_unknown") =>
            {
                self.resolve_by_key(
                    key,
                    if suffix == "/sessions" {
                        Resolver::Admin
                    } else {
                        Resolver::Workspace
                    },
                )
                .await
            }
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
                return Err(resolution_error("operation_indeterminate", None));
            }
            Err(error) => return Err(error),
        };
        match status.state {
            OperationState::Committed => serde_json::from_value(
                status
                    .result
                    .ok_or_else(|| ClientError::Decode("committed receipt has no result".into()))?,
            )
            .map_err(|e| ClientError::Decode(e.to_string())),
            OperationState::Failed => Err(resolution_error(
                "operation_failed",
                Some(status.operation_id),
            )),
            OperationState::Pending => Err(resolution_error(
                "outcome_unknown",
                Some(status.operation_id),
            )),
            OperationState::Indeterminate => Err(resolution_error(
                "operation_indeterminate",
                Some(status.operation_id),
            )),
        }
    }
    pub async fn get_workspace(&self) -> Result<WorkspaceInfo, ClientError> {
        self.get("", vec![]).await
    }
    pub async fn stat(&self, path: Option<&str>) -> Result<InodeAttributes, ClientError> {
        self.get("/stat", path_query(path)).await
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
        self.get("/directory", q).await
    }
    pub async fn read_symlink(&self, path: &str) -> Result<SymlinkContents, ClientError> {
        self.get("/symlinks", path_query(Some(path))).await
    }
    pub async fn find_operation(
        &self,
        key: &IdempotencyKey,
    ) -> Result<OperationStatus, ClientError> {
        self.get(
            "/operations",
            vec![("idempotency_key".into(), key.as_str().into())],
        )
        .await
    }
    pub async fn get_operation(&self, id: Uuid) -> Result<OperationStatus, ClientError> {
        self.get(&format!("/operations/{id}"), vec![]).await
    }
    pub async fn find_admin_operation(
        &self,
        key: &IdempotencyKey,
    ) -> Result<OperationStatus, ClientError> {
        decode(
            self.request(
                Method::GET,
                "/v1/admin-operations",
                vec![("idempotency_key".into(), key.as_str().into())],
                Body::Empty,
                None,
                self.metadata_deadline,
            )
            .await?
            .0,
        )
    }
    pub async fn get_admin_operation(&self, id: Uuid) -> Result<OperationStatus, ClientError> {
        decode(
            self.request(
                Method::GET,
                &format!("/v1/admin-operations/{id}"),
                vec![],
                Body::Empty,
                None,
                self.metadata_deadline,
            )
            .await?
            .0,
        )
    }
    pub async fn create_session(
        &self,
        credential_id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<SessionGrant, ClientError> {
        self.mutation(
            Method::POST,
            "/sessions",
            Body::Json(json!({"credential_id": credential_id, "bind_to_workflow_lease": null})),
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn renew_session(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<SessionGrant, ClientError> {
        self.mutation(
            Method::POST,
            &format!("/sessions/{id}/renew"),
            Body::Empty,
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn revoke_session(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Method::DELETE,
            &format!("/sessions/{id}"),
            Body::Empty,
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn open_handle(
        &self,
        body: &OpenHandle,
        key: &IdempotencyKey,
    ) -> Result<HandleGrant, ClientError> {
        self.mutation(
            Method::POST,
            "/handles",
            Body::Json(json!(body)),
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn renew_handle(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<HandleGrant, ClientError> {
        self.mutation(
            Method::POST,
            &format!("/handles/{id}/renew"),
            Body::Empty,
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn close_handle(
        &self,
        id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Method::DELETE,
            &format!("/handles/{id}"),
            Body::Empty,
            vec![],
            key,
            false,
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
            Method::POST,
            &format!("/handles/{id}/truncate"),
            Body::Json(json!({"size":size})),
            vec![],
            key,
            true,
        )
        .await
    }
    pub async fn fsync(&self, id: Uuid) -> Result<InodeAttributes, ClientError> {
        decode(
            self.request(
                Method::POST,
                &self.workspace_route(&format!("/handles/{id}/fsync")),
                vec![],
                Body::Empty,
                None,
                self.data_deadline,
            )
            .await?
            .0,
        )
    }
    pub async fn read_range(
        &self,
        id: Uuid,
        offset: u64,
        length: u32,
    ) -> Result<(Bytes, i64), ClientError> {
        let (bytes, headers) = self
            .request(
                Method::GET,
                &self.workspace_route(&format!("/handles/{id}/bytes")),
                vec![
                    ("offset".into(), offset.to_string()),
                    ("length".into(), length.to_string()),
                ],
                Body::Empty,
                None,
                self.data_deadline,
            )
            .await?;
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
            Method::PUT,
            &format!("/handles/{id}/bytes"),
            Body::Bytes(bytes),
            vec![("offset".into(), offset.to_string())],
            key,
            true,
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
            Method::POST,
            &format!("/handles/{id}/append"),
            Body::Bytes(bytes),
            vec![],
            key,
            true,
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
            Method::POST,
            "/directories",
            Body::Json(json!({"path":path,"mode":mode})),
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn remove_directory(
        &self,
        path: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Method::DELETE,
            "/directories",
            Body::Empty,
            path_query(Some(path)),
            key,
            false,
        )
        .await
    }
    pub async fn unlink(
        &self,
        path: &str,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Method::DELETE,
            "/files",
            Body::Empty,
            path_query(Some(path)),
            key,
            false,
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
            Method::POST,
            "/links",
            Body::Json(json!({"existing_path":existing_path,"new_path":new_path})),
            vec![],
            key,
            false,
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
            Method::POST,
            "/symlinks",
            Body::Json(json!({"path":path,"target":target})),
            vec![],
            key,
            false,
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
            Method::POST,
            "/rename",
            Body::Json(json!({"from":from,"to":to})),
            vec![],
            key,
            false,
        )
        .await
    }
    pub async fn set_attributes(
        &self,
        attributes: &SetAttributes,
        key: &IdempotencyKey,
    ) -> Result<MutationResult, ClientError> {
        self.mutation(
            Method::PATCH,
            "/attributes",
            Body::Json(json!(attributes)),
            vec![],
            key,
            false,
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
fn resolution_error(code: &str, operation_id: Option<Uuid>) -> ClientError {
    ClientError::Api(ApiFailure {
        status: 409,
        code: code.into(),
        message: "receipt resolution did not establish success".into(),
        trace_id: String::new(),
        operation_id,
        retry_after_ms: None,
    })
}
