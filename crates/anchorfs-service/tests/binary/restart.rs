//! Acknowledged bytes survive real service process death, using only fresh
//! disposable PostgreSQL and MinIO. No container/storage crash is claimed.
use super::*;
use reqwest::{Method, RequestBuilder};
use serde_json::{Value, json};

struct Server {
    child: ChildGuard,
    client: reqwest::Client,
    base: String,
}
impl Server {
    async fn start(url: &str, minio: &minio::Minio) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut command = command();
        with_minio(&mut command, minio);
        let child = ChildGuard(
            command
                .arg("serve")
                .env("ANCHORFS_DATABASE_URL", url)
                .env("ANCHORFS_DURABILITY_PROFILE", "development")
                .env("ANCHORFS_LISTEN_ADDR", address.to_string())
                .env("ANCHORFS_CHUNK_SIZE", "4096")
                .env("ANCHORFS_MAX_REQUEST_BYTES", "4096")
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut server = Self {
            child,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            base: format!("http://{address}"),
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                assert!(
                    server.child.0.try_wait().unwrap().is_none(),
                    "service exited before readiness"
                );
                if let Ok(response) = server
                    .client
                    .get(format!("{}/readyz", server.base))
                    .send()
                    .await
                    && response.status() == 200
                {
                    let ready: Value = response.json().await.unwrap();
                    assert_eq!(ready["backend"], "ok");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap();
        server
    }
    fn request(
        &self,
        method: Method,
        path: &str,
        credential: &Value,
        session: Option<&Value>,
    ) -> RequestBuilder {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(
                "Authorization",
                format!(
                    "Bearer {}.{}",
                    credential["credential_id"].as_str().unwrap(),
                    credential["secret"].as_str().unwrap()
                ),
            );
        if let Some(session) = session {
            request = request
                .header(
                    "X-Anchorfs-Session-Id",
                    session["session_id"].as_str().unwrap(),
                )
                .header(
                    "X-Anchorfs-Session-Epoch",
                    session["session_epoch"].as_i64().unwrap(),
                );
        }
        request
    }
    async fn session(&self, workspace: &str, admin: &Value, worker: &Value, key: &str) -> Value {
        metadata(
            self.request(
                Method::POST,
                &format!("/v1/workspaces/{workspace}/sessions"),
                admin,
                None,
            )
            .header("Idempotency-Key", key)
            .json(&json!({"credential_id":worker["credential_id"]})),
            201,
        )
        .await
    }
    async fn open(
        &self,
        workspace: &str,
        worker: &Value,
        session: &Value,
        create: bool,
        key: &str,
    ) -> Value {
        metadata(self.request(Method::POST,&format!("/v1/workspaces/{workspace}/handles"),worker,Some(session))
            .header("Idempotency-Key",key).json(&json!({"path":"durable","read":true,"write":true,"create":create,"exclusive":false,"truncate":false,"mode":384})),201).await
    }
    async fn read(&self, path: &str, worker: &Value, session: &Value, generation: i64) -> Vec<u8> {
        let mut bytes = Vec::new();
        for offset in [0, 4096] {
            let response = self
                .request(
                    Method::GET,
                    &format!("{path}/bytes?offset={offset}&length=4096"),
                    worker,
                    Some(session),
                )
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["X-Anchorfs-Generation"]
                    .to_str()
                    .unwrap(),
                generation.to_string()
            );
            bytes.extend_from_slice(&response.bytes().await.unwrap());
        }
        bytes
    }
    fn kill(&mut self) {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.child.0.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();
        assert!(!self.child.0.wait().unwrap().success());
    }
}
async fn metadata(request: RequestBuilder, status: u16) -> Value {
    let response = request.send().await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    response.json().await.unwrap()
}

#[tokio::test]
async fn serve_restart_preserves_bytes() {
    let (_database, url) = database(&[]).await;
    let minio = minio::start().await;
    let bootstrap = command()
        .args(["bootstrap-admin", "--uid", "1000", "--gid", "1000"])
        .env("ANCHORFS_DATABASE_URL", &url)
        .env("ANCHORFS_CHUNK_SIZE", "4096")
        .output()
        .unwrap();
    assert!(bootstrap.status.success());
    let admin: Value = serde_json::from_slice(&bootstrap.stdout).unwrap();
    let mut server = Server::start(&url, &minio).await;
    let created=metadata(server.request(Method::POST,"/v1/workspaces",&admin,None)
        .header("Idempotency-Key","workspace").json(&json!({"root_uid":1000,"root_gid":1000,"logical_bytes_limit":1<<20,"logical_inodes_limit":100,"physical_bytes_limit":4<<20,"staging_bytes_limit":1<<20})),201).await;
    let workspace = created["workspace"]["workspace_id"].as_str().unwrap();
    let worker=metadata(server.request(Method::POST,"/v1/credentials",&admin,None)
        .header("Idempotency-Key","worker").json(&json!({"workspace_id":workspace,"kind":"worker","role":"write","uid":1000,"gid":1000,"max_reservation_bytes":1<<20})),201).await;
    let session = server
        .session(workspace, &admin, &worker, "session-a")
        .await;
    let handle = server
        .open(workspace, &worker, &session, true, "handle-a")
        .await;
    let path = format!(
        "/v1/workspaces/{workspace}/handles/{}",
        handle["handle_id"].as_str().unwrap()
    );
    let expected: Vec<_> = (0..6000).map(|i| (i % 251) as u8).collect();
    let mut generation = 0;
    for (offset, end, key) in [(0, 4000, "write-a"), (4000, 6000, "write-b")] {
        let written = metadata(
            server
                .request(
                    Method::PUT,
                    &format!("{path}/bytes?offset={offset}"),
                    &worker,
                    Some(&session),
                )
                .header("Content-Type", "application/octet-stream")
                .header("Idempotency-Key", key)
                .body(expected[offset..end].to_vec()),
            200,
        )
        .await;
        generation = written["generation"].as_i64().unwrap();
    }
    assert_eq!(
        server.read(&path, &worker, &session, generation).await,
        expected
    );
    server.kill();
    drop(server);
    let mut restarted = Server::start(&url, &minio).await;
    let session = restarted
        .session(workspace, &admin, &worker, "session-b")
        .await;
    let handle = restarted
        .open(workspace, &worker, &session, false, "handle-b")
        .await;
    assert_eq!(handle["attributes"]["generation"], generation);
    let path = format!(
        "/v1/workspaces/{workspace}/handles/{}",
        handle["handle_id"].as_str().unwrap()
    );
    assert_eq!(
        restarted.read(&path, &worker, &session, generation).await,
        expected
    );
    let pool = anchorfs_persistence::pool::PgPool::connect(&url)
        .await
        .unwrap();
    let workspace = anchorfs_core::WorkspaceId(workspace.parse().unwrap());
    assert!(
        anchorfs_persistence::invariants::check(&pool, workspace)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        anchorfs_persistence::invariants::check_deployment(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    restarted.kill();
}
