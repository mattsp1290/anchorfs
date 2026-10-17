use crate::{
    error::{ApiError, map_rejection},
    middleware::TraceId,
};
use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::request::Parts,
};
use serde::de::DeserializeOwned;
fn trace(parts: &Parts) -> String {
    parts
        .extensions
        .get::<TraceId>()
        .expect("trace middleware")
        .0
        .clone()
}
pub struct ApiPath<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let id = trace(parts);
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct ApiQuery<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let id = trace(parts);
        // serde_urlencoded otherwise replaces invalid UTF-8. Check the decoded bytes once
        // before typed decoding so malformed paths cannot become replacement-character names.
        if !query_is_utf8(parts.uri.query().unwrap_or("")) {
            return Err(map_rejection(axum::http::StatusCode::BAD_REQUEST, id));
        }

        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct ApiJson<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let id = req
            .extensions()
            .get::<TraceId>()
            .expect("trace middleware")
            .0
            .clone();
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct IdempotencyKey(pub String);
impl<S: Send + Sync> FromRequestParts<S> for IdempotencyKey {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("Idempotency-Key")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 256 && v.bytes().all(|b| b.is_ascii_graphic()));
        value
            .map(|v| Self(v.to_owned()))
            .ok_or_else(|| map_rejection(axum::http::StatusCode::BAD_REQUEST, trace(parts)))
    }
}
pub struct BinaryBody;
impl<S: Send + Sync> FromRequest<S> for BinaryBody {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let id = req
            .extensions()
            .get::<TraceId>()
            .expect("trace middleware")
            .0
            .clone();
        if req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != Some("application/octet-stream")
        {
            return Err(map_rejection(
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                id,
            ));
        }
        axum::body::Bytes::from_request(req, state)
            .await
            .map(|_| Self)
            .map_err(|r| map_rejection(r.status(), id))
    }
}

/// Shape validation only. Stage 1 checks the authenticated actor, workspace, epochs and expiry.
pub struct SessionBinding {
    pub session_id: uuid::Uuid,
    pub session_epoch: i64,
}
fn header<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts.headers.get(name)?.to_str().ok()
}
fn positive_epoch(value: Option<&str>) -> Option<i64> {
    value?.parse::<i64>().ok().filter(|epoch| *epoch > 0)
}
fn workflow_proof(parts: &Parts, required: bool) -> Result<(), ApiError> {
    let token = header(parts, "X-Anchorfs-Workflow-Token");
    let epoch = header(parts, "X-Anchorfs-Workflow-Epoch");
    let absent = !parts.headers.contains_key("X-Anchorfs-Workflow-Token")
        && !parts.headers.contains_key("X-Anchorfs-Workflow-Epoch");
    if (!required && absent)
        || (token.is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            && positive_epoch(epoch).is_some())
    {
        Ok(())
    } else {
        Err(map_rejection(
            axum::http::StatusCode::BAD_REQUEST,
            trace(parts),
        ))
    }
}
impl<S: Send + Sync> FromRequestParts<S> for SessionBinding {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let session_id = header(parts, "X-Anchorfs-Session-Id").and_then(|v| v.parse().ok());
        let session_epoch = positive_epoch(header(parts, "X-Anchorfs-Session-Epoch"));
        workflow_proof(parts, false)?;
        match (session_id, session_epoch) {
            (Some(session_id), Some(session_epoch)) => Ok(Self {
                session_id,
                session_epoch,
            }),
            _ => Err(map_rejection(
                axum::http::StatusCode::BAD_REQUEST,
                trace(parts),
            )),
        }
    }
}
pub struct RequiredWorkflowGrant;
impl<S: Send + Sync> FromRequestParts<S> for RequiredWorkflowGrant {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        workflow_proof(parts, true)?;
        Ok(Self)
    }
}

fn query_is_utf8(query: &str) -> bool {
    let raw = query.as_bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            if i + 2 >= raw.len() {
                return false;
            }
            let hex = |b: u8| char::from(b).to_digit(16);
            let (Some(high), Some(low)) = (hex(raw[i + 1]), hex(raw[i + 2])) else {
                return false;
            };
            decoded.push((high * 16 + low) as u8);
            i += 3;
        } else {
            decoded.push(raw[i]);
            i += 1;
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}
