use crate::{app::AppState, error::ApiError, middleware::TraceId};
use anchorfs_core::{API_PATH_VERSION, ErrorCode};
use anchorfs_persistence::{
    meta,
    settings::{self, DurabilityProfile, DurabilitySettings},
};
use axum::{Extension, Json, extract::State};
use std::time::{Duration, Instant};
/// Leave response construction headroom within the two-second readiness wrapper.
pub fn probe_budget(elapsed: Duration, configured: Duration) -> Option<Duration> {
    let remaining = Duration::from_secs(2)
        .checked_sub(elapsed)?
        .checked_sub(Duration::from_millis(100))?;
    let budget = configured.min(remaining);
    (budget >= Duration::from_millis(100)).then_some(budget)
}
pub async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"alive"}))
}
pub async fn readyz(
    State(state): State<AppState>,
    Extension(trace): Extension<TraceId>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let started = Instant::now();
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
        let backend_error = |message: String| {
            let mut error = ApiError::new(ErrorCode::DependencyUnavailable, &trace.0);
            error.message = format!("backend: {message}");
            error
        };
        // Hold the mutex across a probe to coalesce concurrent readiness polls.
        let mut cache = state.probe_cache.lock().await;
        let result = match cache.as_ref() {
            Some((at, result)) if at.elapsed() < Duration::from_secs(1) => result.clone(),
            _ => {
                let budget = probe_budget(started.elapsed(), state.config.backend_probe_deadline)
                    .ok_or_else(|| {
                    backend_error("no readiness budget left after database checks".into())
                })?;
                let result = match tokio::time::timeout(budget, state.backend.probe()).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("probe deadline exceeded".into()),
                };
                *cache = Some((Instant::now(), result.clone()));
                result
            }
        };
        result.map_err(backend_error)?;
        drop(cache);
        let warnings: Vec<_> = settings
            .unsafe_settings()
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut body = serde_json::json!({"schema_version":versions.schema_version,"object_format_version":versions.object_format_version,"api_version":API_PATH_VERSION,"durability_profile":if state.config.durability_profile==DurabilityProfile::Durable {"durable"}else{"development"},"database":"ok","backend":"ok","checksum_mode":match state.config.s3.checksum_mode { anchorfs_backend::config::ChecksumMode::Sha256 => "sha256", anchorfs_backend::config::ChecksumMode::None => "none" }});
        if !warnings.is_empty() {
            body["durability_warnings"] = serde_json::json!(warnings);
        }
        Ok(Json(body))
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), check)
        .await
        .unwrap_or_else(|_| Err(ApiError::new(ErrorCode::DependencyUnavailable, trace.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_budget_arithmetic() {
        assert_eq!(
            probe_budget(Duration::ZERO, Duration::from_secs(1)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            probe_budget(Duration::from_millis(1500), Duration::from_secs(1)),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            probe_budget(Duration::from_millis(1850), Duration::from_secs(1)),
            None
        );
        assert_eq!(
            probe_budget(Duration::ZERO, Duration::from_millis(100)),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            probe_budget(Duration::from_secs(3), Duration::from_secs(1)),
            None
        );
    }
}
