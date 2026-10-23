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
        let http = http_builder(&endpoint, ca_bundle)?
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .map_err(ClientError::transport)?;
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

/// Hardened `reqwest` builder shared by [`Client::new`] and [`readiness::readiness`]:
/// no proxy, HTTPS-only for https endpoints, no redirects, trust roots only from the PEM bundle
/// when given. Each caller adds its own pooling options and builds.
pub(crate) fn http_builder(
    endpoint: &Endpoint,
    ca_bundle: Option<&[u8]>,
) -> Result<reqwest::ClientBuilder, ClientError> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .https_only(endpoint.is_https())
        .redirect(reqwest::redirect::Policy::none());
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
    Ok(builder)
}

#[cfg(test)]
mod tests;
