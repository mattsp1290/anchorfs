//! Unauthenticated readiness probe: a single `GET /readyz`, never retried.
use crate::{
    endpoint::Endpoint,
    error::ClientError,
    http_builder,
    wire::{ApiErrorBody, ReadyzBody},
};
use std::time::Duration;

/// Outcome of one readiness request that produced an HTTP response.
#[derive(Debug, Clone)]
pub enum Readiness {
    /// HTTP 200 with a readiness document.
    Ready(ReadyzBody),
    /// Any other status; `error` is the parsed API error document when the body is one.
    NotReady {
        status: u16,
        error: Option<ApiErrorBody>,
    },
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
    let http = http_builder(endpoint, ca_bundle)?
        .build()
        .map_err(ClientError::transport)?;
    let response = http
        .get(endpoint.route("/readyz"))
        .header("accept", "application/json")
        .timeout(timeout)
        .send()
        .await
        .map_err(ClientError::transport)?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(ClientError::transport)?;
    if status == 200 {
        let body = serde_json::from_slice::<ReadyzBody>(&bytes)
            .map_err(|_| ClientError::Decode("readyz body".into()))?;
        Ok(Readiness::Ready(body))
    } else {
        Ok(Readiness::NotReady {
            status,
            error: serde_json::from_slice::<ApiErrorBody>(&bytes).ok(),
        })
    }
}
