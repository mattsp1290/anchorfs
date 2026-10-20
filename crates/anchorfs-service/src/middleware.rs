use crate::error::ApiError;
use axum::{
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
#[derive(Clone, Copy)]
pub struct RequestStart(pub std::time::Instant);
#[derive(Clone)]
pub struct TraceId(pub String);
pub async fn request(mut req: Request, next: Next) -> Response {
    req.extensions_mut()
        .insert(RequestStart(std::time::Instant::now()));
    let trace = uuid::Uuid::new_v4().to_string();
    req.extensions_mut().insert(TraceId(trace.clone()));
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".into());
    let method = req.method().clone();
    let mut response = next.run(req).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE
        && response
            .extensions()
            .get::<crate::error::StructuredError>()
            .is_none()
    {
        response = ApiError::new(anchorfs_core::ErrorCode::PayloadTooLarge, &trace).into_response();
    }
    response.headers_mut().insert(
        "X-Anchorfs-Trace-Id",
        HeaderValue::from_str(&trace).expect("UUID header"),
    );
    tracing::info!(method=%method,route=%route,status=response.status().as_u16(),"request completed");
    response
}
/// Route-group deadline; the outer request layer supplies the trace identity.
pub async fn deadline(
    State(deadline): State<std::time::Duration>,
    req: Request,
    next: Next,
) -> Response {
    let trace = req
        .extensions()
        .get::<TraceId>()
        .expect("trace middleware")
        .0
        .clone();
    match tokio::time::timeout(deadline, next.run(req)).await {
        Ok(response) => response,
        Err(_) => {
            let mut error = ApiError::new(anchorfs_core::ErrorCode::DependencyUnavailable, trace);
            error.retry_after_ms = Some(100);
            error.into_response()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn deadline_maps_to_503() {
        let app = Router::new()
            .route(
                "/slow",
                axum::routing::get(|| async {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                std::time::Duration::from_millis(10),
                deadline,
            ))
            .layer(axum::middleware::from_fn(request));
        let response = app
            .oneshot(Request::builder().uri("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(value["code"], "dependency_unavailable");
        assert_eq!(value["retry_after_ms"], 100);
    }
}
