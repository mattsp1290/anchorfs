//! Partial AnchorFS-native API client for mounts and host consumers.
//! Does not wrap workspace creation, credentials, session rebind, workflow leases,
//! read pins or conditional replacement.
pub mod auth;
pub mod endpoint;
pub mod error;
pub mod ops;
pub mod readiness;
pub mod retry;
pub mod wire;

use auth::Authority;
use endpoint::Endpoint;
use error::ClientError;
use std::time::Duration;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: Endpoint,
    workspace: uuid::Uuid,
    authority: Authority,
    metadata_deadline: Duration,
    data_deadline: Duration,
}
impl Client {
    pub fn new(
        endpoint: Endpoint,
        workspace: uuid::Uuid,
        authority: Authority,
        ca_bundle: Option<&[u8]>,
    ) -> Result<Self, ClientError> {
        let http = build_http(&endpoint, ca_bundle, Some(Duration::from_secs(60)))?;
        Ok(Self {
            http,
            endpoint,
            workspace,
            authority,
            metadata_deadline: Duration::from_secs(10),
            data_deadline: Duration::from_secs(60),
        })
    }
    pub fn authority(&self) -> Authority {
        self.authority.clone()
    }
    /// Build a separate client identity. Existing clones and requests are unchanged.
    pub fn with_authority(mut self, authority: Authority) -> Self {
        self.authority = authority;
        self
    }
    pub fn with_deadlines(mut self, metadata: Duration, data: Duration) -> Self {
        self.metadata_deadline = metadata;
        self.data_deadline = data;
        self
    }
}

/// Hardened `reqwest` construction shared by [`Client::new`] and [`readiness::readiness`]:
/// no proxy, HTTPS-only for https endpoints, no redirects, trust roots only from the PEM bundle
/// when given. `pool_idle` is `None` for one-shot callers that keep no connection pool.
pub(crate) fn build_http(
    endpoint: &Endpoint,
    ca_bundle: Option<&[u8]>,
    pool_idle: Option<Duration>,
) -> Result<reqwest::Client, ClientError> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .https_only(endpoint.is_https())
        .redirect(reqwest::redirect::Policy::none());
    if let Some(idle) = pool_idle {
        builder = builder.pool_idle_timeout(idle);
    }
    if let Some(pem) = ca_bundle {
        let certs = reqwest::Certificate::from_pem_bundle(pem)
            .map_err(|_| ClientError::InvalidInput("invalid CA bundle".into()))?;
        if certs.is_empty() {
            return Err(ClientError::InvalidInput("empty CA bundle".into()));
        }
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder.build().map_err(ClientError::transport)
}

#[cfg(test)]
mod tests;
