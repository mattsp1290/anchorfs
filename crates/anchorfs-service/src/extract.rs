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
