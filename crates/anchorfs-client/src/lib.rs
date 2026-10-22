//! Partial AnchorFS-native API client for mounts and host consumers.
//! Does not wrap workspace creation, credentials, session rebind, workflow leases,
//! read pins or conditional replacement.
pub mod auth;
pub mod endpoint;
pub mod error;
pub mod ops;
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
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .https_only(endpoint.is_https())
            .redirect(reqwest::redirect::Policy::none())
            .pool_idle_timeout(Duration::from_secs(60));
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
        Ok(Self {
            http: builder.build().map_err(ClientError::transport)?,
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

#[cfg(test)]
mod tests;
