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
pub enum Answer {
    Metadata(Value),
    Secret(Value),
}
impl Answer {
    pub fn metadata(value: impl serde::Serialize) -> Result<Self, crate::error::ApiError> {
        Ok(Self::Metadata(json(value)?))
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
            response: Answer::Metadata(value),
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
            Answer::Secret(value)
        } else {
            Answer::Metadata(value)
        };
        Ok(Self { stored, response })
    }
}
/// Original admitted authority: self-fencing mutations must not reload their changed epochs.
#[derive(Clone, Copy)]
pub struct Authority<'a> {
    pub actor: &'a anchorfs_persistence::authority::ActorCredential,
    pub session: Option<&'a anchorfs_persistence::authority::ActorSession>,
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
    use anchorfs_persistence::{
        authority::check_publication_time,
        receipts::{self, ReceiptEntry, ReceiptError},
    };
    let entry = receipts::begin(tx, scope, request)
        .await
        .map_err(|error| match error {
            ReceiptError::Database(error) => error.into(),
            ReceiptError::PendingOwner => {
                let mut error = ApiError::new(ErrorCode::OutcomeUnknown, "");
                error.retry_after_ms = Some(100);
                error
            }
        })?;
    let result = match entry {
        ReceiptEntry::Fresh(operation) => {
            let published = mutate(tx, operation).await?;
            check_publication_time(tx, authority.actor, authority.session).await?;
            receipts::commit_result(tx, scope, operation, &published.stored).await?;
            return Ok(Ok(published.response));
        }
        ReceiptEntry::Replay { result, .. } => Ok(Answer::Metadata(result)),
        ReceiptEntry::Mismatch { operation_id } => {
            Err(receipt_error(ErrorCode::IdempotencyMismatch, operation_id))
        }
        ReceiptEntry::Expired { operation_id } => Err(receipt_error(
            ErrorCode::OperationIndeterminate,
            operation_id,
        )),
    };
    check_publication_time(tx, authority.actor, authority.session).await?;
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
pub fn response(
    result: Result<Result<Answer, crate::error::ApiError>, crate::error::ApiError>,
    status: StatusCode,
    trace: &str,
) -> Result<Response, crate::error::ApiError> {
    match result.and_then(|inner| inner) {
        Ok(answer) => {
            let (value, secret) = match answer {
                Answer::Metadata(value) => (value, false),
                Answer::Secret(value) => (value, true),
            };
            let mut response = (status, Json(value)).into_response();
            if secret {
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
