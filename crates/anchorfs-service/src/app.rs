use crate::{config::Config, error::ApiError};
use axum::{Router, extract::DefaultBodyLimit, middleware, response::IntoResponse};
use std::sync::Arc;
use utoipa_axum::router::UtoipaMethodRouterExt;
#[derive(Clone)]
pub struct AppState {
    pub pool: anchorfs_persistence::pool::PgPool,
    pub config: Arc<Config>,
}
/// Register each handler and its documentation together.
pub fn contract_router(config: &Config) -> utoipa_axum::router::OpenApiRouter<AppState> {
    let mut router = utoipa_axum::router::OpenApiRouter::new();
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workspaces::create_workspace).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workspaces::get_workspace).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::credentials::create_credential).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::credentials::rotate_credential).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::credentials::revoke_credential).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::sessions::create_session).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::sessions::renew_session).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::sessions::rebind_session).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::sessions::revoke_session).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(utoipa_axum::routes!(crate::v1::namespace::stat_path).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::list_directory).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::create_directory).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::remove_directory).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(utoipa_axum::routes!(crate::v1::namespace::unlink).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::create_link).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::create_symlink).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::read_symlink).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(utoipa_axum::routes!(crate::v1::namespace::rename).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(
        utoipa_axum::routes!(crate::v1::namespace::set_attributes).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(utoipa_axum::routes!(crate::v1::handles::open_handle).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(
        utoipa_axum::routes!(crate::v1::handles::renew_handle).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::handles::close_handle).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(utoipa_axum::routes!(crate::v1::handles::truncate).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(utoipa_axum::routes!(crate::v1::handles::fsync).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(utoipa_axum::routes!(crate::v1::bytes::read_range).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(utoipa_axum::routes!(crate::v1::bytes::write_at).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(utoipa_axum::routes!(crate::v1::bytes::append).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
    ));
    router = router.routes(utoipa_axum::routes!(crate::v1::bytes::replace_file).layer(
        tower_http::limit::RequestBodyLimitLayer::new(config.max_replace_bytes),
    ));
    router = router.routes(
        utoipa_axum::routes!(crate::v1::pins::create_read_pin).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::pins::release_read_pin).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::operations::get_operation).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::operations::find_operation).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workflows::acquire_workflow_lease).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workflows::renew_workflow_lease).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workflows::release_workflow_lease).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router = router.routes(
        utoipa_axum::routes!(crate::v1::workflows::takeover_workflow_lease).layer(
            tower_http::limit::RequestBodyLimitLayer::new(config.max_request_bytes),
        ),
    );
    router
}
pub fn build_router(state: AppState) -> Router {
    let (router, _) = contract_router(&state.config).split_for_parts();
    router
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::request,
        ))
        .with_state(state)
}
