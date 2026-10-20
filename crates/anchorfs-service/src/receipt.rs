use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value;

/// Emit recursively sorted object keys independently of serde_json feature unification.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    fn emit(value: &Value, out: &mut Vec<u8>) {
        match value {
            Value::Object(map) => {
                out.push(b'{');
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort_unstable();
                for (i, key) in keys.into_iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    out.extend(serde_json::to_vec(key).expect("JSON key"));
                    out.push(b':');
                    emit(&map[key], out);
                }
                out.push(b'}');
            }
            Value::Array(values) => {
                out.push(b'[');
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        out.push(b',');
                    }
                    emit(value, out);
                }
                out.push(b']');
            }
            _ => out.extend(serde_json::to_vec(value).expect("JSON scalar")),
        }
    }
    let mut out = Vec::new();
    emit(value, &mut out);
    out
}
pub fn request_digest(op_type: &str, path_ids: &[&str], body: Option<&Value>) -> [u8; 32] {
    let mut bytes =
        format!("anchorfs-request-v1\0{op_type}\0{}\0", path_ids.join("/")).into_bytes();
    bytes.extend(canonical_json(body.unwrap_or(&Value::Null)));
    anchorfs_core::sha256(bytes).0
}
/// Cache policy is selected from a typed grant before JSON serialization.
/// No Debug implementation: Secret answers contain initial grant material.
#[derive(Debug, Clone, Copy)]
pub enum FollowUp {
    FenceSession {
        workspace: anchorfs_core::WorkspaceId,
        session: anchorfs_core::SessionId,
        operation: anchorfs_core::OperationId,
    },
}
pub struct Answer {
    value: Value,
    secret: bool,
    pub followups: Vec<FollowUp>,
}
impl Answer {
    fn value(value: Value, secret: bool) -> Self {
        Self {
            value,
            secret,
            followups: Vec::new(),
        }
    }
    pub fn metadata(value: impl serde::Serialize) -> Result<Self, crate::error::ApiError> {
        Ok(Self::value(json(value)?, false))
    }
}
pub fn expiry_followups(
    workspace: Option<anchorfs_core::WorkspaceId>,
    tombstoned: Option<anchorfs_persistence::receipts::Tombstoned>,
) -> Vec<FollowUp> {
    match (workspace, tombstoned) {
        (Some(workspace), Some(t)) if t.session.is_some() => vec![FollowUp::FenceSession {
            workspace,
            session: t.session.expect("checked session"),
            operation: t.operation_id,
        }],
        _ => Vec::new(),
    }
}
fn json(value: impl serde::Serialize) -> Result<Value, crate::error::ApiError> {
    serde_json::to_value(value).map_err(|_| {
        crate::error::ApiError::new(anchorfs_core::ErrorCode::DependencyUnavailable, "")
    })
}
/// A fresh operation can expose grant material that its durable receipt must omit.
pub struct Published {
    stored: Value,
    response: Answer,
}
impl Published {
    pub fn metadata(value: impl serde::Serialize) -> Result<Self, crate::error::ApiError> {
        let value = json(value)?;
        Ok(Self {
            stored: value.clone(),
            response: Answer::value(value, false),
        })
    }
    pub fn credential(
        mut grant: crate::dto::IssuedCredential,
    ) -> Result<Self, crate::error::ApiError> {
        let secret = grant.secret.take();
        let stored = json(&grant)?;
        let contains_secret = secret.is_some();
        grant.secret = secret;
        let value = json(grant)?;
        let response = if contains_secret {
            Answer::value(value, true)
        } else {
            Answer::value(value, false)
        };
        Ok(Self { stored, response })
    }
}
/// Original admitted authority: self-fencing mutations must not reload their changed epochs.
#[derive(Clone, Copy)]
pub struct Authority<'a> {
    pub actor: &'a anchorfs_persistence::authority::ActorCredential,
    pub session: Option<&'a anchorfs_persistence::authority::ActorSession>,
    pub workflow: Option<WorkflowAuthority<'a>>,
}
#[derive(Clone, Copy)]
pub struct WorkflowAuthority<'a> {
    pub workspace: &'a anchorfs_persistence::authority::WorkspaceLock,
    pub proof: Option<&'a anchorfs_persistence::authority::WorkflowProof>,
    pub mode: anchorfs_persistence::authority::LeaseMode,
}
impl Authority<'_> {
    pub(crate) async fn check_publication(
        self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), crate::error::ApiError> {
        if let Some(workflow) = self.workflow {
            let session = self
                .session
                .ok_or_else(|| crate::error::ApiError::new(anchorfs_core::ErrorCode::Fenced, ""))?;
            anchorfs_persistence::authority::check_workflow(
                tx,
                workflow.workspace,
                self.actor,
                session,
                workflow.proof,
                workflow.mode,
            )
            .await?;
        }
        anchorfs_persistence::authority::check_publication_time(tx, self.actor, self.session)
            .await?;
        Ok(())
    }
}
pub type PublicationFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Published, crate::error::ApiError>> + Send + 'a>,
>;

/// One lifecycle owns receipt dispatch, final authority time, and fresh result storage.
/// The outer control runner commits Ok(Err(_)) tombstones and rolls back Err(_).
/// Persistence helpers retain their own time checks for direct transaction callers;
/// this final service boundary also covers receipt waits and complete orchestration.
/// A direct persistence caller adding later waits must likewise guard its own
/// final publication point; an earlier helper check cannot freeze database time.
pub async fn execute(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    authority: Authority<'_>,
    scope: anchorfs_persistence::receipts::ReceiptScope,
    request: anchorfs_persistence::receipts::ReceiptRequest<'_>,
    mutate: impl for<'a> FnOnce(
        &'a mut sqlx::Transaction<'static, sqlx::Postgres>,
        anchorfs_core::OperationId,
    ) -> PublicationFuture<'a>
    + Send,
) -> Result<Result<Answer, crate::error::ApiError>, crate::error::ApiError> {
    use crate::error::ApiError;
    use anchorfs_core::ErrorCode;
    use anchorfs_persistence::receipts::{self, ReceiptEntry, ReceiptError};
    let entry = receipts::begin(tx, scope, request)
        .await
        .map_err(|error| match error {
            ReceiptError::Database(error) => error.into(),
            ReceiptError::NotOwner => ApiError::new(ErrorCode::Fenced, ""),
            ReceiptError::PendingOwner => {
                let mut error = ApiError::new(ErrorCode::OutcomeUnknown, "");
                error.retry_after_ms = Some(100);
                error
            }
        })?;
    let result = match entry {
        ReceiptEntry::Fresh {
            operation_id: operation,
            ..
        } => {
            let published = mutate(tx, operation).await?;
            authority.check_publication(tx).await?;
            if !receipts::commit_result(tx, scope, operation, &published.stored).await? {
                return Err(ApiError::new(ErrorCode::Fenced, ""));
            }
            return Ok(Ok(published.response));
        }
        ReceiptEntry::Replay { result, .. } => Ok(Answer::value(result, false)),
        ReceiptEntry::Mismatch { operation_id } => {
            Err(receipt_error(ErrorCode::IdempotencyMismatch, operation_id))
        }
        ReceiptEntry::Expired {
            operation_id,
            tombstoned,
        } => {
            let mut error = receipt_error(ErrorCode::OperationIndeterminate, operation_id);
            let workspace = match scope {
                receipts::ReceiptScope::Workspace { workspace, .. } => Some(workspace),
                _ => None,
            };
            error.followups = expiry_followups(workspace, tombstoned);
            Err(error)
        }
        ReceiptEntry::Failed { operation_id } => {
            Err(receipt_error(ErrorCode::OperationFailed, operation_id))
        }
        ReceiptEntry::Pending { operation_id, .. } => {
            let mut error = receipt_error(ErrorCode::OutcomeUnknown, operation_id);
            error.retry_after_ms = Some(100);
            Err(error)
        }
    };
    authority.check_publication(tx).await?;
    Ok(result)
}
fn receipt_error(
    code: anchorfs_core::ErrorCode,
    operation: anchorfs_core::OperationId,
) -> crate::error::ApiError {
    let mut error = crate::error::ApiError::new(code, "");
    error.operation_id = Some(operation);
    error
}
/// Commit-first errors carry the route's trace after the outer transaction succeeds.
pub async fn finish(
    state: &crate::app::AppState,
    result: Result<Result<Answer, crate::error::ApiError>, crate::error::ApiError>,
    status: StatusCode,
    trace: &str,
) -> Result<Response, crate::error::ApiError> {
    let mut result = result.and_then(|inner| inner);
    let followups = match &mut result {
        Ok(answer) => std::mem::take(&mut answer.followups),
        Err(error) => std::mem::take(&mut error.followups),
    };
    for followup in followups {
        use crate::pipeline::faults::{FaultAction, FaultPoint};
        match state.faults.at(FaultPoint::BeforeFollowUp, None) {
            FaultAction::Crash => break,
            FaultAction::Wait(mut gate) => {
                while !*gate.borrow_and_update() {
                    if gate.changed().await.is_err() {
                        break;
                    }
                }
            }
            FaultAction::Continue => (),
        }
        let followup_result = crate::control::run(
            state,
            crate::middleware::RequestStart(std::time::Instant::now()),
            crate::middleware::TraceId(trace.into()),
            crate::control::Kind::Mutation,
            move |tx, _| {
                Box::pin(async move {
                    match followup {
                        FollowUp::FenceSession {
                            workspace,
                            session,
                            operation,
                        } => {
                            anchorfs_persistence::sessions::fence_for_expired_receipt(
                                tx, workspace, session, operation,
                            )
                            .await?;
                        }
                    }
                    Ok(())
                })
            },
        )
        .await;
        if followup_result.is_err() {
            tracing::warn!(
                trace_id = trace,
                "receipt session-fence follow-up failed; next encounter retries"
            );
        }
    }
    match result {
        Ok(answer) => {
            let mut response = (status, Json(answer.value)).into_response();
            if answer.secret {
                response.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
            }
            Ok(response)
        }
        Err(mut error) => {
            if error.trace_id.is_empty() {
                error.trace_id = trace.into();
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_order_is_recursive() {
        let a = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":0}"#).unwrap();
        let b = serde_json::from_str(r#"{"a":0,"z":[{"a":1,"b":2}]}"#).unwrap();
        assert_eq!(canonical_json(&a), br#"{"a":0,"z":[{"a":1,"b":2}]}"#);
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(
            request_digest("op", &["a"], Some(&a)),
            request_digest("op", &["a"], Some(&b))
        );
        assert_ne!(
            request_digest("op", &["a"], Some(&a)),
            request_digest("op", &["b"], Some(&a))
        );
    }
}
