//! Executable tests use only synthetic credentials and disposable PostgreSQL data.
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
#[path = "common/minio.rs"]
mod minio;
#[path = "binary/restart.rs"]
mod restart;
const DIGEST: &str = "sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3";
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_anchorfs-service"));
    for (name, _) in std::env::vars().filter(|(name, _)| name.starts_with("ANCHORFS_")) {
        c.env_remove(name);
    }
    c.env("ANCHORFS_DEPLOYMENT_ID", "test")
        .env("ANCHORFS_CURSOR_SIGNING_KEY", "00".repeat(32))
        .env("RUST_LOG", "error");
    c
}
async fn database(settings: &[&str]) -> (ContainerAsync<Postgres>, String) {
    let mut args = vec!["postgres".to_owned()];
    for setting in settings {
        args.extend(["-c".into(), setting.to_string()]);
    }
    let c = Postgres::default()
        .with_fsync_enabled()
        .with_tag(format!("17@{DIGEST}"))
        .with_cmd(args)
        .start()
        .await
        .unwrap();
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        c.get_host().await.unwrap(),
        c.get_host_port_ipv4(5432).await.unwrap()
    );
    (c, url)
}
async fn sql(c: &ContainerAsync<Postgres>, statement: &str) -> String {
    let mut result = c
        .exec(testcontainers::core::ExecCommand::new([
            "psql",
            "-U",
            "postgres",
            "-At",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            statement,
        ]))
        .await
        .unwrap();
    let output = result.stdout_to_vec().await.unwrap();
    assert_eq!(result.exit_code().await.unwrap(), Some(0));
    String::from_utf8(output).unwrap()
}
fn spawn(url: &str, profile: &str, address: &str) -> ChildGuard {
    ChildGuard(
        command()
            .arg("serve")
            .env("ANCHORFS_DATABASE_URL", url)
            .env("ANCHORFS_DURABILITY_PROFILE", profile)
            .env("ANCHORFS_LISTEN_ADDR", address)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    )
}
async fn exit(child: &mut ChildGuard) -> (i32, String) {
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            let mut stderr = String::new();
            child
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            return (status.code().unwrap(), stderr);
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "binary did not exit"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn starts_and_stops(url: &str, profile: &str, bind_all: bool) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let address = format!("{}:{port}", if bind_all { "0.0.0.0" } else { "127.0.0.1" });
    let minio = if profile == "development" {
        Some(minio::start().await)
    } else {
        None
    };
    let mut cmd = command();
    if let Some(minio) = &minio {
        with_minio(&mut cmd, minio);
    }
    let mut child = ChildGuard(
        cmd.arg("serve")
            .env("ANCHORFS_DATABASE_URL", url)
            .env("ANCHORFS_DURABILITY_PROFILE", profile)
            .env("ANCHORFS_LISTEN_ADDR", &address)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let start = Instant::now();
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "binary exited before readiness"
        );
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/readyz"))
            .send()
            .await
            && response.status() == if profile == "development" { 200 } else { 503 }
        {
            let v: serde_json::Value = response.json().await.unwrap();
            if profile == "development" {
                assert_eq!(v["schema_version"], 1);
                assert_eq!(v["backend"], "ok");
            } else {
                assert_eq!(v["code"], "dependency_unavailable");
                assert!(v["message"].as_str().unwrap().starts_with("backend:"));
                assert_eq!(
                    client
                        .get(format!("http://127.0.0.1:{port}/healthz"))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    200
                );
            }
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "binary failed readiness"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.0.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let stop = Instant::now();
    let (code, stderr) = exit(&mut child).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stop.elapsed() < Duration::from_secs(10));
}
#[tokio::test]
async fn serve_exits_on_fsync_off_durable() {
    let (c, url) = database(&["fsync=off"]).await;
    let (code, error) = exit(&mut spawn(&url, "durable", "127.0.0.1:0")).await;
    assert_eq!(code, 3);
    assert!(error.contains("fsync"));
    assert_eq!(
        sql(&c, "SELECT to_regclass('public._sqlx_migrations') IS NULL")
            .await
            .trim(),
        "t"
    );
    starts_and_stops(&url, "development", false).await;
}
#[tokio::test]
async fn serve_exits_on_server_sync_commit_off() {
    let (_c, url) = database(&["synchronous_commit=off"]).await;
    let (code, error) = exit(&mut spawn(&url, "durable", "127.0.0.1:0")).await;
    assert_eq!(code, 3);
    assert!(error.contains("synchronous_commit"));
}
#[tokio::test]
async fn serve_refuses_non_loopback_plaintext_durable() {
    let mut child = spawn(
        "postgres://unused:unused@127.0.0.1:1/unused",
        "durable",
        "0.0.0.0:0",
    );
    let (code, error) = exit(&mut child).await;
    assert_eq!(code, 7);
    assert!(error.contains("requires TLS"));
    let (_c, url) = database(&[]).await;
    starts_and_stops(&url, "development", true).await;
}
#[tokio::test]
async fn serve_starts_and_reports_ready() {
    let (_c, url) = database(&[]).await;
    starts_and_stops(&url, "durable", false).await;
}
#[tokio::test]
async fn serve_refuses_newer_database() {
    let (c, url) = database(&[]).await;
    sql(
        &c,
        "CREATE TABLE _sqlx_migrations(version bigint, success boolean); INSERT INTO _sqlx_migrations VALUES(9999,true)",
    )
    .await;
    let (code, error) = exit(&mut spawn(&url, "durable", "127.0.0.1:0")).await;
    assert_eq!(code, 4);
    assert!(error.contains("newer"));
}
#[test]
fn openapi_command_matches_snapshot() {
    let output = command()
        .args(["openapi", "--format", "yaml"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/api/openapi-v1.yaml"
        ))
        .unwrap()
        .trim_end()
    );
}

fn with_minio(cmd: &mut Command, minio: &minio::Minio) {
    cmd.env("ANCHORFS_S3_ENDPOINT", minio.endpoint.as_str())
        .env("ANCHORFS_S3_BUCKET", minio::BUCKET)
        .env("ANCHORFS_S3_REGION", "us-east-1")
        .env("ANCHORFS_S3_ALLOW_PLAINTEXT", "true")
        .env("ANCHORFS_S3_CREDENTIAL_SOURCE", "static")
        .env("ANCHORFS_S3_ACCESS_KEY_ID", minio::ROOT_USER)
        .env("ANCHORFS_S3_SECRET_ACCESS_KEY", minio::ROOT_PASSWORD);
}
#[tokio::test]
async fn serve_rejects_plaintext_s3_durable() {
    let output = command()
        .arg("serve")
        .env(
            "ANCHORFS_DATABASE_URL",
            "postgres://unused:unused@127.0.0.1:1/unused",
        )
        .env("ANCHORFS_S3_ALLOW_PLAINTEXT", "true")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("S3_ALLOW_PLAINTEXT"));
}
#[test]
fn config_checksum_mode_and_probe_deadline() {
    // Subprocesses isolate environment parsing without process-wide environment mutations.
    for (mode, deadline, error) in [
        ("sha256", "1000", None),
        ("none", "100", None),
        ("md5", "1000", Some("S3_CHECKSUM_MODE")),
        ("sha256", "50", Some("LIMITS")),
        ("sha256", "5000", Some("LIMITS")),
    ] {
        let output = command()
            .arg("serve")
            .env(
                "ANCHORFS_DATABASE_URL",
                "postgres://unused:unused@127.0.0.1:1/unused",
            )
            .env("ANCHORFS_S3_CHECKSUM_MODE", mode)
            .env("ANCHORFS_BACKEND_PROBE_DEADLINE_MS", deadline)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(error.unwrap_or("database connection failed")),
            "{stderr}"
        );
    }
    let defaults = anchorfs_service::config::Config::contract_defaults();
    assert_eq!(
        defaults.s3.checksum_mode,
        anchorfs_backend::config::ChecksumMode::Sha256
    );
    assert_eq!(defaults.backend_probe_deadline, Duration::from_secs(1));
}

#[test]
fn config_byte_path_bounds() {
    for (vars, valid) in [
        (vec![("WORKFLOW_LEASE_TTL_MS", "0")], false),
        (vec![("CHUNK_SIZE", "1024")], false),
        (vec![("CHUNK_SIZE", "8388608")], false),
        (
            vec![("CHUNK_SIZE", "4096"), ("MAX_REQUEST_BYTES", "4097")],
            false,
        ),
        (vec![("MAX_DATA_OPS", "7")], false),
        (vec![("UPLOAD_RESERVATION_TTL_MS", "362000")], false),
        (vec![("UPLOAD_RESERVATION_TTL_MS", "86400000")], false),
        (vec![("READ_PIN_TTL_MS", "0")], false),
        (vec![("MAX_READ_PINS_PER_SESSION", "0")], false),
        (vec![("MAX_RETRY_ATTEMPTS", "0")], false),
        (
            vec![("CHUNK_SIZE", "4096"), ("MAX_REQUEST_BYTES", "4096")],
            true,
        ),
        (
            vec![("DB_MAX_CONNECTIONS", "1"), ("MAX_DATA_OPS", "1")],
            true,
        ),
        (
            vec![("DB_MAX_CONNECTIONS", "2"), ("MAX_DATA_OPS", "2")],
            false,
        ),
    ] {
        let mut cmd = command();
        cmd.arg("serve").env(
            "ANCHORFS_DATABASE_URL",
            "postgres://unused:unused@127.0.0.1:1/unused",
        );
        for (name, value) in &vars {
            cmd.env(format!("ANCHORFS_{name}"), value);
        }
        let output = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{vars:?}");
        if valid {
            assert!(
                stderr.contains("database connection failed"),
                "{vars:?}: {stderr}"
            );
        } else {
            assert_eq!(output.status.code(), Some(2), "{vars:?}: {stderr}");
            assert!(
                stderr.contains("invalid or missing ANCHORFS_"),
                "{vars:?}: {stderr}"
            );
        }
    }
}

#[tokio::test]
async fn serve_refuses_changed_chunk_size() {
    let (container, url) = database(&[]).await;
    let bootstrap = || {
        let mut cmd = command();
        cmd.args(["bootstrap-admin", "--uid", "1000", "--gid", "1000"])
            .env("ANCHORFS_DATABASE_URL", &url)
            .env("ANCHORFS_CHUNK_SIZE", "4096");
        cmd
    };
    assert!(bootstrap().output().unwrap().status.success());
    assert_eq!(
        sql(&container, "SELECT chunk_size FROM schema_meta")
            .await
            .trim(),
        "4096"
    );
    let minio = minio::start().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut cmd = command();
    with_minio(&mut cmd, &minio);
    let mut child = ChildGuard(
        cmd.arg("serve")
            .env("ANCHORFS_DATABASE_URL", &url)
            .env("ANCHORFS_DURABILITY_PROFILE", "development")
            .env("ANCHORFS_LISTEN_ADDR", address.to_string())
            .env("ANCHORFS_CHUNK_SIZE", "4096")
            .env("ANCHORFS_MAX_REQUEST_BYTES", "4096")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "service exited before readiness"
            );
            if let Ok(response) = client.get(format!("http://{address}/readyz")).send().await
                && response.status() == 200
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    // Bootstrap identity comparison must succeed again; rotate the synthetic
    // admin first because bootstrap intentionally rejects an existing admin.
    sql(&container, "UPDATE credentials SET revoked_at=clock_timestamp(),epoch=epoch+1 WHERE kind='administrator'").await;
    assert!(bootstrap().output().unwrap().status.success());
    let mut cmd = command();
    with_minio(&mut cmd, &minio);
    let output = cmd
        .arg("serve")
        .env("ANCHORFS_DATABASE_URL", &url)
        .env("ANCHORFS_DURABILITY_PROFILE", "development")
        .env("ANCHORFS_LISTEN_ADDR", address.to_string())
        .env("ANCHORFS_CHUNK_SIZE", "8192")
        .env("ANCHORFS_MAX_REQUEST_BYTES", "8192")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&output.stderr).contains("database chunk size 4096"));
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert_eq!(
        sql(&container, "SELECT chunk_size FROM schema_meta")
            .await
            .trim(),
        "4096"
    );
}

#[tokio::test]
async fn bootstrap_admin_once() {
    let (container, url) = database(&[]).await;
    let first = command()
        .args(["bootstrap-admin", "--uid", "1000", "--gid", "1000"])
        .env("ANCHORFS_DATABASE_URL", &url)
        .env("ANCHORFS_DEPLOYMENT_PHYSICAL_BYTES_LIMIT", "12345")
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&first.stdout).lines().count(), 1);
    let grant: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(grant["credential_id"].as_str().unwrap().len(), 36);
    let secret = grant["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(
        secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert!(!String::from_utf8_lossy(&first.stderr).contains(secret));
    let second = command()
        .args(["bootstrap-admin", "--uid", "1000", "--gid", "1000"])
        .env("ANCHORFS_DATABASE_URL", &url)
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(8));
    assert!(second.stdout.is_empty());
    assert_eq!(
        sql(&container, "SELECT count(*) FROM credentials")
            .await
            .trim(),
        "1"
    );
    assert_eq!(
        sql(
            &container,
            "SELECT physical_bytes_limit FROM deployment_quotas"
        )
        .await
        .trim(),
        "12345"
    );
    // Re-initialization with different defaults must preserve the bootstrap quota.
    let pool = anchorfs_persistence::pool::PgPool::connect(&url)
        .await
        .unwrap();
    anchorfs_persistence::meta::initialize(
        &pool,
        &anchorfs_core::DeploymentId::parse("test").unwrap(),
        &anchorfs_service::config::Config::contract_defaults().quota_config(),
        anchorfs_core::Limits::default().chunk_size,
    )
    .await
    .unwrap();
    assert_eq!(
        sql(
            &container,
            "SELECT physical_bytes_limit FROM deployment_quotas"
        )
        .await
        .trim(),
        "12345"
    );
    sql(&container,"UPDATE credentials SET revoked_at=clock_timestamp(),epoch=epoch+1 WHERE revoked_at IS NULL").await;
    let recovery = command()
        .args(["bootstrap-admin", "--uid", "1000", "--gid", "1000"])
        .env("ANCHORFS_DATABASE_URL", &url)
        .output()
        .unwrap();
    assert!(
        recovery.status.success(),
        "{}",
        String::from_utf8_lossy(&recovery.stderr)
    );
    assert_eq!(
        sql(
            &container,
            "SELECT count(*) FROM credentials WHERE revoked_at IS NULL"
        )
        .await
        .trim(),
        "1"
    );
}

#[path = "binary/tls.rs"]
mod tls_tests;

#[test]
fn probe_argument_validation() {
    for args in [
        &["--url", "http://127.0.0.1:1"][..],
        &["--url", "https://127.0.0.1:1/readyz"],
        &["--url", "https://127.0.0.1:1", "--timeout-ms", "0"],
        &[
            "--url",
            "https://127.0.0.1:1",
            "--ca-bundle",
            "/nonexistent/ca.pem",
        ],
    ] {
        let output = command().arg("probe").args(args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
