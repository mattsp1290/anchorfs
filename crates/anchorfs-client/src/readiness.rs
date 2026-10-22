//! Unauthenticated readiness probe: a single `GET /readyz`, never retried.
use crate::{
    build_http,
    endpoint::Endpoint,
    error::ClientError,
    wire::{ApiErrorBody, ReadyzBody},
};
use std::time::Duration;

/// Outcome of one readiness request that produced an HTTP response.
/// A 200 yields `ready`; any other status yields `error` when the body is an API error document.
#[derive(Debug, Clone)]
pub struct Readiness {
    pub status: u16,
    pub ready: Option<ReadyzBody>,
    pub error: Option<ApiErrorBody>,
}

/// Requests `<origin>/readyz` without credentials. Transport failures (including TLS trust
/// failures and the whole-request `timeout`) are errors; any HTTP response is reported as data.
pub async fn readiness(
    endpoint: &Endpoint,
    ca_bundle: Option<&[u8]>,
    timeout: Duration,
) -> Result<Readiness, ClientError> {
    if timeout.is_zero() {
        return Err(ClientError::InvalidInput("timeout must be positive".into()));
    }
    let http = build_http(endpoint, ca_bundle, None)?;
    let response = http
        .get(endpoint.route("/readyz"))
        .header("accept", "application/json")
        .timeout(timeout)
        .send()
        .await
        .map_err(ClientError::transport)?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(ClientError::transport)?;
    let (ready, error) = if status == 200 {
        let body = serde_json::from_slice::<ReadyzBody>(&bytes)
            .map_err(|_| ClientError::Decode("readyz body".into()))?;
        (Some(body), None)
    } else {
        (None, serde_json::from_slice::<ApiErrorBody>(&bytes).ok())
    };
    Ok(Readiness {
        status,
        ready,
        error,
    })
}
