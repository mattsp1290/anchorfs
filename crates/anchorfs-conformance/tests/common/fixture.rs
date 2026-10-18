//! HTTP acceptance from approved Stage 1a decisions, with disposable metadata only.
use crate::common;
pub use crate::common::{
    client::{Client, Credential},
    fake_backend::FakeBackend,
};
pub use anchorfs_service::{
    app::{AppState, build_router},
    config::Config,
};
pub use serde_json::{Value, json};
pub use sqlx::PgPool;
pub use std::{sync::Arc, time::Duration};
pub use uuid::Uuid;

pub struct Fixture {
    pub _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    pub pool: PgPool,
    pub client: Client,
    pub backend: Arc<FakeBackend>,
    pub admin: Credential,
    pub host: Credential,
    pub worker: Credential,
    pub workspace: Uuid,
    pub host_session: Value,
    pub worker_session: Value,
}
pub fn workspace_body() -> Value {
    json!({"root_uid":1000,"root_gid":1000,"logical_bytes_limit":10000,"logical_inodes_limit":100,"physical_bytes_limit":20000,"staging_bytes_limit":10000})
}
pub fn credential_body(workspace: Uuid, kind: &str) -> Value {
    json!({"workspace_id":workspace,"kind":kind,"role":"write","uid":1000,"gid":1000,"max_reservation_bytes":4096})
}
impl Fixture {
    pub async fn new() -> Self {
        Self::with(workspace_body(), |_| {}).await
    }
    pub async fn with(body: Value, configure: impl FnOnce(&mut Config)) -> Self {
        let (container, pool, _) = common::database(&[]).await;
        let (mut state, backend) = common::state_fake(pool.clone());
        let mut config = (*state.config).clone();
        configure(&mut config);
        state.cursor_key =
            anchorfs_core::decode_hex_32(config.cursor_signing_key.expose()).expect("test key");
        state.config = Arc::new(config);
        let client = Client {
            app: build_router(state),
        };
        let issued =
            anchorfs_service::bootstrap::create_administrator(&pool, 1000, 1000, 4096, None)
                .await
                .unwrap();
        let admin = Credential {
            id: issued.id.0,
            secret: issued.secret.expose().clone(),
        };
        let created = client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&admin),
                None,
                Some("workspace"),
                Some(body),
            )
            .await
            .status(201);
        let workspace =
            Uuid::parse_str(created.body["workspace"]["workspace_id"].as_str().unwrap()).unwrap();
        let host = Credential::from_grant(
            &client
                .call(
                    "POST",
                    "/v1/credentials",
                    Some(&admin),
                    None,
                    Some("host"),
                    Some(credential_body(workspace, "host")),
                )
                .await
                .status(201)
                .body,
        );
        let worker = Credential::from_grant(
            &client
                .call(
                    "POST",
                    "/v1/credentials",
                    Some(&host),
                    None,
                    Some("worker"),
                    Some(credential_body(workspace, "worker")),
                )
                .await
                .status(201)
                .body,
        );
        let path = format!("/v1/workspaces/{workspace}/sessions");
        let host_session = client
            .call(
                "POST",
                &path,
                Some(&host),
                None,
                Some("host-session"),
                Some(json!({"credential_id":host.id})),
            )
            .await
            .status(201)
            .body;
        let worker_session = client
            .call(
                "POST",
                &path,
                Some(&host),
                None,
                Some("worker-session"),
                Some(json!({"credential_id":worker.id})),
            )
            .await
            .status(201)
            .body;
        Self {
            _container: container,
            pool,
            client,
            backend,
            admin,
            host,
            worker,
            workspace,
            host_session,
            worker_session,
        }
    }
    pub async fn invariants(&self) {
        assert!(
            anchorfs_persistence::invariants::check(
                &self.pool,
                anchorfs_core::WorkspaceId(self.workspace)
            )
            .await
            .unwrap()
            .is_empty()
        );
        self.backend.untouched();
    }
    pub async fn credential(&self, kind: &str, role: &str, uid: u32, gid: u32) -> Credential {
        let body = json!({"workspace_id":self.workspace,"kind":kind,"role":role,"uid":uid,"gid":gid,"max_reservation_bytes":4096});
        Credential::from_grant(
            &self
                .client
                .call(
                    "POST",
                    "/v1/credentials",
                    Some(&self.admin),
                    None,
                    Some(&Uuid::new_v4().to_string()),
                    Some(body),
                )
                .await
                .status(201)
                .body,
        )
    }
    pub async fn session_for(&self, credential: &Credential) -> Value {
        self.client
            .call(
                "POST",
                &format!("{}/sessions", self.workspace_path()),
                Some(&self.admin),
                None,
                Some(&Uuid::new_v4().to_string()),
                Some(json!({"credential_id":credential.id})),
            )
            .await
            .status(201)
            .body
    }
    pub fn client_with_deadline(&self, millis: u64) -> Client {
        let mut config = Config::contract_defaults();
        config.metadata_call_deadline = Duration::from_millis(millis);
        Client {
            app: build_router(AppState {
                cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
                    .expect("validated cursor signing key"),
                pool: self.pool.clone(),
                config: Arc::new(config),
                backend: self.backend.clone(),
                probe_cache: Default::default(),
                control_permits: Arc::new(tokio::sync::Semaphore::new(2)),
            }),
        }
    }
    pub fn workspace_path(&self) -> String {
        format!("/v1/workspaces/{}", self.workspace)
    }
    pub fn session_path(&self, session: &Value, action: &str) -> String {
        format!(
            "{}/sessions/{}{action}",
            self.workspace_path(),
            session["session_id"].as_str().unwrap()
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.backend.untouched();
    }
}

pub async fn wait_for_lock(pool: &PgPool, query_fragment: &str) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND state='active' AND wait_event_type='Lock' AND position($1 in query)>0)").bind(query_fragment).fetch_one(pool).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}
pub async fn wait_until_expired(pool: &PgPool, expiry: time::OffsetDateTime) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=$1")
                .bind(expiry)
                .fetch_one(pool)
                .await
                .unwrap();
            if expired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
