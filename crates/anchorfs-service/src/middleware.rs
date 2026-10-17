use crate::{app::AppState, error::ApiError};
use axum::{
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
#[derive(Clone)]
pub struct TraceId(pub String);
pub async fn request(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let trace = uuid::Uuid::new_v4().to_string();
    req.extensions_mut().insert(TraceId(trace.clone()));
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".into());
    let method = req.method().clone();
    let deadline =
        if route.ends_with("/bytes") || route.ends_with("/append") || route.ends_with("/files") {
            state.config.data_call_deadline
        } else {
            state.config.metadata_call_deadline
        };
    let mut response = match tokio::time::timeout(deadline, next.run(req)).await {
        Ok(response) => response,
        Err(_) => {
            let mut error = ApiError::new(anchorfs_core::ErrorCode::DependencyUnavailable, &trace);
            error.retry_after_ms = Some(100);
            error.into_response()
        }
    };
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
        response = ApiError::new(anchorfs_core::ErrorCode::PayloadTooLarge, &trace).into_response();
    }
    response.headers_mut().insert(
        "X-Anchorfs-Trace-Id",
        HeaderValue::from_str(&trace).expect("UUID header"),
    );
    tracing::info!(method=%method,route=%route,status=response.status().as_u16(),"request completed");
    response
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
        let mut config = crate::config::Config::contract_defaults();
        config.metadata_call_deadline = std::time::Duration::from_millis(10);
        let state = AppState {
            pool: anchorfs_persistence::pool::PgPool::connect_lazy(
                "postgres://unused:unused@127.0.0.1:1/unused",
            )
            .unwrap(),
            config: config.shared(),
        };
        let app = Router::new()
            .route(
                "/slow",
                axum::routing::get(|| async {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }),
            )
            .layer(axum::middleware::from_fn_with_state(state, request));
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
