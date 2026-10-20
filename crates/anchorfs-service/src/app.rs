use crate::{config::Config, error::ApiError};
use axum::{Router, extract::DefaultBodyLimit, middleware, response::IntoResponse};
use std::{sync::Arc, time::Duration};
use utoipa_axum::router::OpenApiRouter;
pub type ProbeCache = Arc<tokio::sync::Mutex<Option<(std::time::Instant, Result<(), String>)>>>;
#[derive(Clone)]
pub struct AppState {
    pub cursor_key: [u8; 32],
    pub control_permits: Arc<tokio::sync::Semaphore>,
    pub data_permits: Arc<tokio::sync::Semaphore>,
    pub faults: Arc<dyn crate::pipeline::faults::FaultInjector>,
    pub pool: anchorfs_persistence::pool::PgPool,
    pub config: Arc<Config>,
    pub backend: Arc<dyn anchorfs_backend::traits::ObjectBackend>,
    pub probe_cache: ProbeCache,
}

impl AppState {
    pub fn production(
        config: Config,
        pool: anchorfs_persistence::pool::PgPool,
        backend: Arc<dyn anchorfs_backend::traits::ObjectBackend>,
    ) -> Self {
        Self {
            cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
                .expect("validated cursor signing key"),
            control_permits: Arc::new(tokio::sync::Semaphore::new(
                config.db_max_connections.saturating_sub(2).max(1) as usize,
            )),
            data_permits: Arc::new(tokio::sync::Semaphore::new(config.max_data_ops as usize)),
            faults: Arc::new(crate::pipeline::faults::NoFaults),
            pool,
            config: config.shared(),
            backend,
            probe_cache: Default::default(),
        }
    }
}

/// Policies belong to the route groups, independently of URL spelling or handler order.
fn with_policy(
    router: OpenApiRouter<AppState>,
    limit: usize,
    deadline: Duration,
) -> OpenApiRouter<AppState> {
    #[cfg(test)]
    let router = router.layer(middleware::from_fn(tests::delay_request));
    router
        .layer(tower_http::limit::RequestBodyLimitLayer::new(limit))
        .layer(middleware::from_fn_with_state(
            deadline,
            crate::middleware::deadline,
        ))
}

/// Register each handler and its document together, applying each shared policy once.
pub fn contract_router(config: &Config) -> OpenApiRouter<AppState> {
    let metadata = OpenApiRouter::new()
        .routes(utoipa_axum::routes!(
            crate::v1::workspaces::create_workspace
        ))
        .routes(utoipa_axum::routes!(crate::v1::workspaces::get_workspace))
        .routes(utoipa_axum::routes!(
            crate::v1::credentials::create_credential
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::credentials::rotate_credential
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::credentials::revoke_credential
        ))
        .routes(utoipa_axum::routes!(crate::v1::sessions::create_session))
        .routes(utoipa_axum::routes!(crate::v1::sessions::renew_session))
        .routes(utoipa_axum::routes!(crate::v1::sessions::rebind_session))
        .routes(utoipa_axum::routes!(crate::v1::sessions::revoke_session))
        .routes(utoipa_axum::routes!(crate::v1::namespace::stat_path))
        .routes(utoipa_axum::routes!(crate::v1::namespace::list_directory))
        .routes(utoipa_axum::routes!(crate::v1::namespace::create_directory))
        .routes(utoipa_axum::routes!(crate::v1::namespace::remove_directory))
        .routes(utoipa_axum::routes!(crate::v1::namespace::unlink))
        .routes(utoipa_axum::routes!(crate::v1::namespace::create_link))
        .routes(utoipa_axum::routes!(crate::v1::namespace::create_symlink))
        .routes(utoipa_axum::routes!(crate::v1::namespace::read_symlink))
        .routes(utoipa_axum::routes!(crate::v1::namespace::rename))
        .routes(utoipa_axum::routes!(crate::v1::namespace::set_attributes))
        .routes(utoipa_axum::routes!(crate::v1::handles::open_handle))
        .routes(utoipa_axum::routes!(crate::v1::handles::renew_handle))
        .routes(utoipa_axum::routes!(crate::v1::handles::close_handle))
        .routes(utoipa_axum::routes!(crate::v1::handles::truncate))
        .routes(utoipa_axum::routes!(crate::v1::handles::fsync))
        .routes(utoipa_axum::routes!(crate::v1::pins::create_read_pin))
        .routes(utoipa_axum::routes!(crate::v1::pins::release_read_pin))
        .routes(utoipa_axum::routes!(crate::v1::operations::get_operation))
        .routes(utoipa_axum::routes!(crate::v1::operations::find_operation))
        .routes(utoipa_axum::routes!(
            crate::v1::workflows::acquire_workflow_lease
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::workflows::renew_workflow_lease
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::workflows::release_workflow_lease
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::workflows::takeover_workflow_lease
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::operations::get_admin_operation
        ))
        .routes(utoipa_axum::routes!(
            crate::v1::operations::find_admin_operation
        ));
    let metadata = with_policy(
        metadata,
        config.max_request_bytes,
        config.metadata_call_deadline,
    );
    let data = OpenApiRouter::new()
        .routes(utoipa_axum::routes!(crate::v1::bytes::read_range))
        .routes(utoipa_axum::routes!(crate::v1::bytes::write_at))
        .routes(utoipa_axum::routes!(crate::v1::bytes::append));
    let data = with_policy(data, config.max_request_bytes, config.data_call_deadline);
    let replacement =
        OpenApiRouter::new().routes(utoipa_axum::routes!(crate::v1::bytes::replace_file));
    let replacement = with_policy(
        replacement,
        config.max_replace_bytes,
        config.data_call_deadline,
    );
    metadata.merge(data).merge(replacement)
}
pub fn build_router(state: AppState) -> Router {
    assert!(
        Some(state.cursor_key)
            == anchorfs_core::decode_hex_32(state.config.cursor_signing_key.expose()),
        "invalid cursor signing key configuration"
    );
    let (router, _) = contract_router(&state.config).split_for_parts();
    let auxiliary = Router::new()
        .route("/healthz", axum::routing::get(crate::health::healthz))
        .route("/readyz", axum::routing::get(crate::health::readyz))
        .route(
            "/openapi.json",
            axum::routing::get(|| async { axum::Json(crate::api_doc::document()) }),
        )
        .route(
            "/openapi.yaml",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/yaml")],
                    crate::api_doc::document()
                        .to_yaml()
                        .expect("OpenAPI serialization"),
                )
            }),
        )
        .layer(middleware::from_fn_with_state(
            state.config.metadata_call_deadline,
            crate::middleware::deadline,
        ));
    router
        .merge(auxiliary)
        .fallback(
            |axum::Extension(trace): axum::Extension<crate::middleware::TraceId>| async move {
                ApiError::new(anchorfs_core::ErrorCode::NotFound, trace.0).into_response()
            },
        )
        .method_not_allowed_fallback(
            |axum::Extension(trace): axum::Extension<crate::middleware::TraceId>| async move {
                ApiError::new(anchorfs_core::ErrorCode::MethodNotAllowed, trace.0).into_response()
            },
        )
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn(crate::middleware::request))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn production_state_installs_no_faults() {
        use crate::pipeline::faults::{FaultAction, FaultPoint};
        let mut config = Config::contract_defaults();
        config.max_data_ops = 3;
        let backend = Arc::new(anchorfs_backend::s3::S3Backend::from_config(&config.s3).unwrap());
        let pool = anchorfs_persistence::pool::PgPool::connect_lazy(
            "postgres://unused:unused@127.0.0.1:1/unused",
        )
        .unwrap();
        let state = AppState::production(config, pool, backend);
        assert_eq!(state.data_permits.available_permits(), 3);
        for point in [
            FaultPoint::AfterPeek,
            FaultPoint::BeforeGet,
            FaultPoint::BeforeReserve,
            FaultPoint::AfterReserve,
            FaultPoint::BeforeUpload,
            FaultPoint::AfterUpload,
            FaultPoint::BeforePublish,
            FaultPoint::AfterPublish,
            FaultPoint::BeforeFinalize,
            FaultPoint::BeforeFollowUp,
        ] {
            assert!(matches!(
                state.faults.at(point, None),
                FaultAction::Continue
            ));
        }
    }
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        extract::Request,
        http::StatusCode,
        middleware::Next,
        response::Response,
    };
    use tower::ServiceExt;
    // Test-only delay inside each real policy group; no production header or route is added.
    pub(super) async fn delay_request(req: Request, next: Next) -> Response {
        if req.headers().contains_key("X-Anchorfs-Test-Delay") {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        next.run(req).await
    }
    #[tokio::test]
    async fn deadline_policy_groups_include_shared_file_methods() {
        let id = "00000000-0000-4000-8000-000000000001";
        for (metadata, data) in [(10, 1000), (1000, 10)] {
            let mut config = Config::contract_defaults();
            config.metadata_call_deadline = Duration::from_millis(metadata);
            config.data_call_deadline = Duration::from_millis(data);
            let state = AppState {
                cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
                    .expect("validated cursor signing key"),
                control_permits: std::sync::Arc::new(tokio::sync::Semaphore::new(6)),
                pool: anchorfs_persistence::pool::PgPool::connect_lazy(
                    "postgres://unused:unused@127.0.0.1:1/unused",
                )
                .unwrap(),
                backend: Arc::new(
                    anchorfs_backend::s3::S3Backend::from_config(&config.s3).unwrap(),
                ),
                probe_cache: Default::default(),
                data_permits: Arc::new(tokio::sync::Semaphore::new(6)),
                faults: Arc::new(crate::pipeline::faults::NoFaults),
                config: config.shared(),
            };
            let app = build_router(state);
            for (method, path, is_data) in [
                ("GET", format!("/v1/workspaces/{id}"), false),
                (
                    "PUT",
                    format!("/v1/workspaces/{id}/handles/{id}/bytes?offset=0"),
                    true,
                ),
                (
                    "POST",
                    format!("/v1/workspaces/{id}/handles/{id}/append"),
                    true,
                ),
                (
                    "PUT",
                    format!("/v1/workspaces/{id}/files?path=file&expected_generation=1"),
                    true,
                ),
                (
                    "DELETE",
                    format!("/v1/workspaces/{id}/files?path=file"),
                    false,
                ),
            ] {
                let request = Request::builder()
                    .method(method)
                    .uri(path.clone())
                    .header("X-Anchorfs-Test-Delay", "true")
                    .header("Idempotency-Key", "deadline-test")
                    .header("Content-Length", "1")
                    .header("X-Anchorfs-Content-SHA256", "ab".repeat(32))
                    .header("X-Anchorfs-Session-Id", id)
                    .header("X-Anchorfs-Session-Epoch", "1")
                    .header("content-type", "application/octet-stream")
                    .body(Body::from(vec![0]))
                    .unwrap();
                let response = app.clone().oneshot(request).await.unwrap();
                let short = if is_data { data } else { metadata } < 60;
                assert_eq!(
                    response.status(),
                    if short {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::UNAUTHORIZED
                    },
                    "{method} {path}"
                );
                let trace = response.headers()["X-Anchorfs-Trace-Id"]
                    .to_str()
                    .unwrap()
                    .to_owned();
                let error: serde_json::Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(error["trace_id"], trace);
                assert_eq!(
                    error["code"],
                    if short {
                        "dependency_unavailable"
                    } else {
                        "unauthenticated"
                    }
                );
                if short {
                    assert_eq!(error["retry_after_ms"], 100);
                }
            }
        }
    }
}
