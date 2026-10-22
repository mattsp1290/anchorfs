//! Startup-only TLS configuration and the shared HTTP/1.1 server lifecycle.
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::{future::Future, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct TlsConfig(Arc<rustls::ServerConfig>);
impl std::fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsConfig(<redacted>)")
    }
}
pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot read {0}")]
    Read(PathBuf),
    #[error("cannot parse {0}")]
    Parse(&'static str),
    #[error("cannot build TLS configuration")]
    Build,
}
pub fn load(paths: &TlsPaths) -> Result<TlsConfig, TlsError> {
    let cert = std::fs::read(&paths.cert).map_err(|_| TlsError::Read(paths.cert.clone()))?;
    let key = std::fs::read(&paths.key).map_err(|_| TlsError::Read(paths.key.clone()))?;
    server_config(&cert, &key)
}
pub fn server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<TlsConfig, TlsError> {
    let certs = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::Parse("certificate"))?;
    if certs.is_empty() {
        return Err(TlsError::Parse("certificate"));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|_| TlsError::Parse("private key"))?;
    // Other workspace clients can enable multiple providers and TLS 1.2.
    // Select both the provider and protocol explicitly rather than using global defaults.
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| TlsError::Build)?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|_| TlsError::Build)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsConfig(Arc::new(config)))
}

pub struct ServerHandle(axum_server::Handle<SocketAddr>);
impl ServerHandle {
    pub fn graceful_shutdown(&self, duration: Option<Duration>) {
        self.0.graceful_shutdown(duration);
    }
    pub fn shutdown(&self) {
        self.0.shutdown();
    }
    pub async fn listening(&self) -> Option<SocketAddr> {
        self.0.listening().await
    }
}
pub fn serve(
    listener: std::net::TcpListener,
    router: axum::Router,
    tls: Option<TlsConfig>,
    handshake_timeout: Duration,
) -> std::io::Result<(ServerHandle, impl Future<Output = std::io::Result<()>>)> {
    let handle = axum_server::Handle::new();
    let make_service = router.into_make_service();
    // Construct before returning so listener conversion errors remain startup errors.
    let server: std::pin::Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>> =
        if let Some(config) = tls {
            let config = RustlsConfig::from_config(config.0);
            Box::pin(
                axum_server::from_tcp_rustls(listener, config.clone())?
                    .acceptor(RustlsAcceptor::new(config).handshake_timeout(handshake_timeout))
                    .http1_only()
                    .handle(handle.clone())
                    .serve(make_service),
            )
        } else {
            Box::pin(
                axum_server::from_tcp(listener)?
                    .http1_only()
                    .handle(handle.clone())
                    .serve(make_service),
            )
        };
    Ok((ServerHandle(handle), server))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_config_sets_http1_alpn_and_rejects_bad_input() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let config = server_config(cert.pem().as_bytes(), key.serialize_pem().as_bytes()).unwrap();
        assert_eq!(config.0.alpn_protocols, [b"http/1.1"]);
        assert_eq!(format!("{config:?}"), "TlsConfig(<redacted>)");
        let other = rcgen::KeyPair::generate().unwrap();
        assert!(matches!(
            server_config(cert.pem().as_bytes(), other.serialize_pem().as_bytes()),
            Err(TlsError::Build)
        ));
        assert!(matches!(
            server_config(b"", key.serialize_pem().as_bytes()),
            Err(TlsError::Parse("certificate"))
        ));
        assert!(matches!(
            server_config(cert.pem().as_bytes(), b"invalid"),
            Err(TlsError::Parse("private key"))
        ));
    }
}
