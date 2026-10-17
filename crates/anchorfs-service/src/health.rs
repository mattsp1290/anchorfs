use crate::{app::AppState, error::ApiError, middleware::TraceId};
use anchorfs_core::{API_PATH_VERSION, ErrorCode};
use anchorfs_persistence::{
    meta,
    settings::{self, DurabilityProfile, DurabilitySettings},
};
use axum::{Extension, Json, extract::State};
pub async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"alive"}))
}
pub async fn readyz(
    State(state): State<AppState>,
    Extension(trace): Extension<TraceId>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let check = async {
        let versions = meta::read_versions(&state.pool)
            .await
            .map_err(|_| ApiError::new(ErrorCode::DependencyUnavailable, &trace.0))?;
        meta::check_versions(&versions)
            .map_err(|_| ApiError::new(ErrorCode::SchemaVersionMismatch, &trace.0))?;
        let settings = DurabilitySettings::read(&state.pool)
            .await
            .map_err(|_| ApiError::new(ErrorCode::DependencyUnavailable, &trace.0))?;
        settings
            .check(state.config.durability_profile)
            .map_err(|err| {
                let mut error = ApiError::new(ErrorCode::UnsafeDatabaseSettings, &trace.0);
                error.message = err.to_string();
                error
            })?;
        let encoding = settings::read_server_encoding(&state.pool)
            .await
            .map_err(|_| ApiError::new(ErrorCode::DependencyUnavailable, &trace.0))?;
        settings::check_encoding(&encoding)
            .map_err(|_| ApiError::new(ErrorCode::DependencyUnavailable, &trace.0))?;
        let warnings: Vec<_> = settings
            .unsafe_settings()
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut body = serde_json::json!({"schema_version":versions.schema_version,"object_format_version":versions.object_format_version,"api_version":API_PATH_VERSION,"durability_profile":if state.config.durability_profile==DurabilityProfile::Durable {"durable"}else{"development"},"database":"ok","backend":"not_checked"});
        if !warnings.is_empty() {
            body["durability_warnings"] = serde_json::json!(warnings);
        }
        Ok(Json(body))
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), check)
        .await
        .unwrap_or_else(|_| Err(ApiError::new(ErrorCode::DependencyUnavailable, trace.0)))
}
