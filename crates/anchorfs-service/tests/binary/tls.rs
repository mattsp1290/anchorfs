use super::*;
#[path = "../../../anchorfs-conformance/tests/common/tls.rs"]
mod pki;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct TlsProcess {
    child: ChildGuard,
    client: reqwest::Client,
    endpoint: String,
    _files: pki::TempPki,
    _database: ContainerAsync<Postgres>,
    _minio: Option<minio::Minio>,
}
impl TlsProcess {
    async fn start(development: bool) -> Self {
        let (db, url) = database(&[]).await;
        let files = pki::TempPki::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let minio = if development {
            Some(minio::start().await)
        } else {
            None
        };
        let mut cmd = command();
        if let Some(minio) = &minio {
            with_minio(&mut cmd, minio);
        }
        let child = ChildGuard(
            cmd.arg("serve")
                .env("ANCHORFS_DATABASE_URL", url)
                .env(
                    "ANCHORFS_DURABILITY_PROFILE",
                    if development {
                        "development"
                    } else {
                        "durable"
                    },
                )
                .env("ANCHORFS_LISTEN_ADDR", format!("0.0.0.0:{port}"))
                .env("ANCHORFS_TLS_CERT_PATH", &files.files.cert)
                .env("ANCHORFS_TLS_KEY_PATH", &files.files.key)
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let client = client(&files.pki.ca_pem).build().unwrap();
        let mut this = Self {
            child,
            client,
            endpoint: format!("https://127.0.0.1:{port}"),
            _files: files,
            _database: db,
            _minio: minio,
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                assert!(
                    this.child.0.try_wait().unwrap().is_none(),
                    "TLS binary exited before readiness"
                );
                if let Ok(r) = this
                    .client
                    .get(format!("{}/readyz", this.endpoint))
                    .send()
                    .await
                    && r.status() == if development { 200 } else { 503 }
                {
                    let body: serde_json::Value = r.json().await.unwrap();
                    if development {
                        assert_eq!(body["tls"], true);
                        assert_eq!(body["backend"], "ok");
                    } else {
                        assert_eq!(body["code"], "dependency_unavailable");
                        assert!(body["message"].as_str().unwrap().starts_with("backend:"));
                        assert!(body.get("tls").is_none());
                    }
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap();
        this.health().await;
        this
    }
    async fn health(&self) {
        assert_eq!(
            self.client
                .get(format!("{}/healthz", self.endpoint))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    fn tls_stream(&self) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
        use rustls_pki_types::{CertificateDer, pem::PemObject};
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from_pem_slice(&self._files.pki.ca_pem).unwrap())
            .unwrap();
        let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let conn = rustls::ClientConnection::new(
            std::sync::Arc::new(config),
            "localhost".try_into().unwrap(),
        )
        .unwrap();
        let socket =
            std::net::TcpStream::connect(self.endpoint.strip_prefix("https://").unwrap()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        rustls::StreamOwned::new(conn, socket)
    }
    async fn stop(mut self) {
        // Hold an explicitly verified, fully consumed keep-alive request open
        // throughout SIGTERM, independently of reqwest pool policy.
        use std::io::{BufRead, Write};
        let mut idle = std::io::BufReader::new(self.tls_stream());
        idle.get_mut()
            .write_all(
                b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
            )
            .unwrap();
        idle.get_mut().flush().unwrap();
        let mut line = String::new();
        idle.read_line(&mut line).unwrap();
        assert!(line.starts_with("HTTP/1.1 200"), "{line}");
        let mut length = None;
        loop {
            line.clear();
            assert!(idle.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("health response length")];
        std::io::Read::read_exact(&mut idle, &mut body).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["status"],
            "alive"
        );
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.child.0.id() as i32),
            nix::sys::signal::Signal::SIGTERM,
        )
        .unwrap();
        let start = Instant::now();
        let (code, stderr) = exit(&mut self.child).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.contains("listening on https://"), "{stderr}");
        assert!(start.elapsed() < Duration::from_secs(10));
        drop(idle);
    }
}
fn client(ca: &[u8]) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .add_root_certificate(reqwest::Certificate::from_pem(ca).unwrap())
        .timeout(Duration::from_secs(5))
}
#[tokio::test]
async fn serve_tls_durable_non_loopback_reports_ready() {
    TlsProcess::start(false).await.stop().await;
}
#[tokio::test]
async fn serve_tls_development_ready_with_minio() {
    TlsProcess::start(true).await.stop().await;
}
#[tokio::test]
async fn serve_tls_rejects_plaintext_and_tls12() {
    let server = TlsProcess::start(false).await;
    server.health().await;
    let addr = server.endpoint.strip_prefix("https://").unwrap();
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut buffer = [0; 512];
    let n = match tokio::time::timeout(Duration::from_secs(5), tcp.read(&mut buffer))
        .await
        .unwrap()
    {
        Ok(n) => n,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            0
        }
        Err(e) => panic!("unexpected plaintext probe error: {e}"),
    };
    assert!(!buffer[..n].starts_with(b"HTTP/"));
    assert!(
        n == 0 || buffer[0] == 0x15,
        "expected TLS alert or EOF: {:?}",
        &buffer[..n]
    );
    let tls12 = client(&server._files.pki.ca_pem)
        .max_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .unwrap();
    let error = tls12
        .get(format!("{}/healthz", server.endpoint))
        .send()
        .await
        .unwrap_err();
    assert!(!error.is_builder(), "{error}");
    let wrong = client(&server._files.pki.other_ca_pem).build().unwrap();
    let error = wrong
        .get(format!("{}/healthz", server.endpoint))
        .send()
        .await
        .unwrap_err();
    assert!(!error.is_builder(), "{error}");
    server.health().await;
    server.stop().await;
}
#[test]
fn serve_tls_config_errors() {
    let files = pki::TempPki::new();
    let missing = files.files.key.with_extension("missing");
    for (cert, key, expected) in [
        (Some(&files.files.cert), None, "set together"),
        (None, Some(&files.files.key), "set together"),
        (
            Some(&files.files.cert),
            Some(&missing),
            "invalid TLS certificate or key",
        ),
        (
            Some(&missing),
            Some(&files.files.key),
            "invalid TLS certificate or key",
        ),
        (
            Some(&files.files.key),
            Some(&files.files.key),
            "invalid TLS certificate or key",
        ),
        (
            Some(&files.files.other_ca),
            Some(&files.files.key),
            "invalid TLS certificate or key",
        ),
        (
            Some(&files.files.cert),
            Some(&files.files.ca),
            "invalid TLS certificate or key",
        ),
    ] {
        for database in [false, true] {
            let mut cmd = command();
            cmd.arg("serve");
            if let Some(cert) = cert {
                cmd.env("ANCHORFS_TLS_CERT_PATH", cert);
            }
            if let Some(key) = key {
                cmd.env("ANCHORFS_TLS_KEY_PATH", key);
            }
            if database {
                cmd.env(
                    "ANCHORFS_DATABASE_URL",
                    "postgres://unused:unused@127.0.0.1:1/unused",
                );
            }
            let output = cmd.output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(2), "{stderr}");
            assert!(stderr.contains(expected), "{stderr}");
            assert!(!stderr.contains("database connection"), "{stderr}");
        }
    }
}

#[tokio::test]
async fn serve_tls_rejects_http2_prior_knowledge() {
    use std::io::Write;
    let server = TlsProcess::start(false).await;
    server.health().await;
    let mut stream = server.tls_stream();
    stream
        .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
        .unwrap();
    stream.flush().unwrap();
    let mut bytes = [0; 512];
    match std::io::Read::read(&mut stream, &mut bytes) {
        Ok(n) => assert!(
            n == 0 || bytes[..n].starts_with(b"HTTP/1.1"),
            "expected HTTP/1 rejection or EOF, got {:?}",
            &bytes[..n]
        ),
        Err(e) => assert!(
            matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::UnexpectedEof
            ),
            "{e}"
        ),
    }
    server.health().await;
    server.stop().await;
}

#[tokio::test]
async fn probe_reports_ready_over_tls_and_rejects_wrong_ca() {
    let server = TlsProcess::start(true).await;
    let probe = |ca: &std::path::Path, extra: &[&str]| {
        let output = command()
            .args(["probe", "--url", &server.endpoint, "--ca-bundle"])
            .arg(ca)
            .args(extra)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, stdout, stderr) = probe(&server._files.files.ca, &["--require-tls"]);
    assert_eq!(code, Some(0), "{stderr}");
    let line: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(line["status"], 200);
    assert_eq!(line["tls"], true);
    assert_eq!(line["database"], "ok");
    assert_eq!(line["backend"], "ok");
    let (code, _, stderr) = probe(&server._files.files.other_ca, &["--require-tls"]);
    assert_eq!(code, Some(2), "{stderr}");
    let (code, _, stderr) = probe(&server._files.files.ca, &["--require-durable"]);
    assert_eq!(code, Some(1), "{stderr}");
    server.stop().await;
}
