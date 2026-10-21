//! Disposable service on a dedicated runtime; all identities follow the caller.
use anchorfs_client::{
    Client,
    auth::{Authority, Bearer, SessionAuthority},
    endpoint::Endpoint,
    retry::IdempotencyKey,
    wire::{HandleGrant, OpenHandle},
};
use anchorfs_core::{DeploymentId, Secret, SessionId, WorkspaceId};
use anchorfs_persistence::{
    meta::{self, DeploymentQuotaConfig},
    migrate,
    pool::{self, PoolConfig},
};
use anchorfs_service::{
    app::{AppState, build_router},
    config::Config,
    pipeline::faults::{FaultInjector, NoFaults},
};
use serde_json::{Value, json};
use std::{future::Future, sync::Arc, time::Duration};
use testcontainers::{ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

pub const POSTGRES_IMAGE_DIGEST: &str =
    "sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3";
#[derive(Clone)]
pub struct Credential {
    pub id: Uuid,
    pub bearer: String,
}
impl Credential {
    pub fn from(value: &Value) -> Self {
        let id = Uuid::parse_str(value["credential_id"].as_str().unwrap()).unwrap();
        Self {
            id,
            bearer: format!("{id}.{}", value["secret"].as_str().unwrap()),
        }
    }
}
pub struct Fixture {
    pub rt: tokio::runtime::Handle,
    pub endpoint: String,
    pub pool: sqlx::PgPool,
    pub workspace: Uuid,
    pub host: Credential,
    pub worker: Credential,
    pub admin: Credential,

    pub session: Value,
    pub host_session: Value,
    pub api: Client,
    pub config: Config,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<std::thread::JoinHandle<()>>,
}
struct Ready {
    endpoint: String,
    pool: sqlx::PgPool,
    workspace: Uuid,
    host: Credential,
    worker: Credential,
    admin: Credential,

    session: Value,
    host_session: Value,
    api: Client,
    config: Config,
    rt: tokio::runtime::Handle,
}
impl Fixture {
    pub fn new() -> Self {
        Self::with(|_| {}, 1 << 30, Arc::new(NoFaults))
    }
    pub fn with(
        configure: impl FnOnce(&mut Config) + Send + 'static,
        quota: u64,
        faults: Arc<dyn FaultInjector>,
    ) -> Self {
        super::mountpoint::require_fuse();
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        assert_ne!(uid, 0, "mount tests require a non-root user");
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let join = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let service = start_service(
                    configure,
                    quota,
                    faults,
                    anchorfs_core::Identity { uid, gid },
                    runtime.handle().clone(),
                )
                .await;
                let pool = service.ready.pool.clone();
                send.send(service.ready)
                    .unwrap_or_else(|_| panic!("fixture receiver disappeared"));
                let _ = stopped.await;
                service.task.abort();
                pool.close().await;
                drop(service.container);
            });
        });
        let ready = receive
            .recv_timeout(Duration::from_secs(120))
            .expect("service fixture startup");
        Self {
            rt: ready.rt,
            endpoint: ready.endpoint,
            pool: ready.pool,
            workspace: ready.workspace,
            host: ready.host,
            worker: ready.worker,
            admin: ready.admin,

            session: ready.session,
            host_session: ready.host_session,
            api: ready.api,
            config: ready.config,
            stop: Some(stop),
            join: Some(join),
        }
    }
    pub fn run<T>(&self, future: impl Future<Output = T>) -> T {
        self.rt.block_on(future)
    }
    pub fn raw(
        &self,
        actor: &Credential,
        session: Option<&Value>,
        method: &str,
        suffix: &str,
        body: Option<Value>,
    ) -> Value {
        self.run(raw(
            &self.endpoint,
            actor,
            session,
            method,
            &format!("/v1/workspaces/{}{suffix}", self.workspace),
            body,
        ))
    }
    pub fn open(&self, path: &str, create: bool) -> HandleGrant {
        self.run(self.api.open_handle(
            &OpenHandle {
                path: path.into(),
                read: true,
                write: true,
                create,
                exclusive: false,
                truncate: false,
                mode: Some(0o644),
            },
            &IdempotencyKey::new(),
        ))
        .unwrap()
    }
    pub fn put(&self, path: &str, data: &[u8]) {
        let handle = self.open(path, true);
        self.run(self.api.write_at(
            handle.handle_id,
            0,
            bytes::Bytes::copy_from_slice(data),
            &IdempotencyKey::new(),
        ))
        .unwrap();
        self.run(
            self.api
                .close_handle(handle.handle_id, &IdempotencyKey::new()),
        )
        .unwrap();
    }
    pub fn read(&self, path: &str) -> Vec<u8> {
        let h = self
            .run(self.api.open_handle(
                &OpenHandle {
                    path: path.into(),
                    read: true,
                    write: false,
                    create: false,
                    exclusive: false,
                    truncate: false,
                    mode: None,
                },
                &IdempotencyKey::new(),
            ))
            .unwrap();
        let size = h.attributes.size;
        let mut bytes = Vec::new();
        while (bytes.len() as u64) < size {
            let chunk = self
                .run(self.api.read_range(
                    h.handle_id,
                    bytes.len() as u64,
                    (size - bytes.len() as u64).min(4 << 20) as u32,
                ))
                .unwrap()
                .0;
            assert!(!chunk.is_empty());
            bytes.extend_from_slice(&chunk);
        }
        self.run(self.api.close_handle(h.handle_id, &IdempotencyKey::new()))
            .unwrap();
        bytes
    }
    pub fn sql(&self, sql: &'static str) {
        self.run(sqlx::query(sql).bind(self.workspace).execute(&self.pool))
            .unwrap();
    }
    pub fn receipt_count(&self) -> i64 {
        self.run(
            sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
                .bind(self.workspace)
                .fetch_one(&self.pool),
        )
        .unwrap()
    }
    pub fn count(&self, op: &str) -> i64 {
        self.run(sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND op_type=$2 AND state='committed'").bind(self.workspace).bind(op).fetch_one(&self.pool)).unwrap()
    }
    pub fn invariants(&self) {
        assert!(
            self.run(anchorfs_persistence::invariants::check(
                &self.pool,
                WorkspaceId(self.workspace)
            ))
            .unwrap()
            .is_empty()
        );
        assert!(
            self.run(anchorfs_persistence::invariants::check_deployment(
                &self.pool
            ))
            .unwrap()
            .is_empty()
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(join) = self.join.take()
            && let Err(e) = join.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(e);
        }
    }
}
pub fn native(
    endpoint: &str,
    workspace: Uuid,
    credential: &Credential,
    session: Option<&Value>,
) -> Client {
    Client::new(
        Endpoint::parse(endpoint, true).unwrap(),
        workspace,
        Authority {
            bearer: Bearer::parse(&credential.bearer).unwrap(),
            session: session.map(|s| SessionAuthority {
                session_id: SessionId(Uuid::parse_str(s["session_id"].as_str().unwrap()).unwrap()),
                session_epoch: s["session_epoch"].as_i64().unwrap(),
            }),
            workflow: None,
        },
        None,
    )
    .unwrap()
}
async fn issue(
    endpoint: &str,
    issuer: &Credential,
    workspace: Uuid,
    kind: &str,
    role: &str,
    uid: u32,
    gid: u32,
) -> Credential {
    Credential::from(&raw(endpoint,issuer,None,"POST","/v1/credentials",Some(json!({"workspace_id":workspace,"kind":kind,"role":role,"uid":uid,"gid":gid,"max_reservation_bytes":1u64<<30}))).await)
}
pub async fn raw(
    endpoint: &str,
    credential: &Credential,
    session: Option<&Value>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Value {
    let mut request = reqwest::Client::new()
        .request(method.parse().unwrap(), format!("{endpoint}{path}"))
        .bearer_auth(&credential.bearer)
        .header("Idempotency-Key", Uuid::new_v4().to_string())
        .timeout(Duration::from_secs(30));
    if let Some(s) = session {
        request = request
            .header("X-Anchorfs-Session-Id", s["session_id"].as_str().unwrap())
            .header(
                "X-Anchorfs-Session-Epoch",
                s["session_epoch"].as_i64().unwrap(),
            );
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.json::<Value>().await.unwrap();
    assert!(status.is_success(), "{method} {path}: {status} {body}");
    body
}

pub fn lease_call(f: &Fixture, method: &str, suffix: &str, token: &str, epoch: i64) -> Value {
    f.run(async {
        let response = reqwest::Client::new()
            .request(
                method.parse().unwrap(),
                format!("{}/v1/workspaces/{}{suffix}", f.endpoint, f.workspace),
            )
            .bearer_auth(&f.host.bearer)
            .header(
                "X-Anchorfs-Session-Id",
                f.host_session["session_id"].as_str().unwrap(),
            )
            .header(
                "X-Anchorfs-Session-Epoch",
                f.host_session["session_epoch"].as_i64().unwrap(),
            )
            .header("X-Anchorfs-Workflow-Token", token)
            .header("X-Anchorfs-Workflow-Epoch", epoch)
            .header("Idempotency-Key", Uuid::new_v4().to_string())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.json::<Value>().await.unwrap();
        assert!(status.is_success(), "{status}: {body}");
        body
    })
}

struct RunningService {
    ready: Ready,
    container: testcontainers::ContainerAsync<Postgres>,
    task: tokio::task::JoinHandle<()>,
}
async fn start_service(
    configure: impl FnOnce(&mut Config),
    quota: u64,
    faults: Arc<dyn FaultInjector>,
    identity: anchorfs_core::Identity,
    rt: tokio::runtime::Handle,
) -> RunningService {
    let container = Postgres::default()
        .with_fsync_enabled()
        .with_tag(format!("17@{POSTGRES_IMAGE_DIGEST}"))
        .start()
        .await
        .unwrap();
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.unwrap(),
        container.get_host_port_ipv4(5432).await.unwrap(),
    );
    let pool = pool::connect(&PoolConfig {
        url: Secret::new(url),
        max_connections: 16,
        acquire_timeout: Duration::from_secs(5),
    })
    .await
    .unwrap();
    migrate::apply(&pool).await.unwrap();
    let minio = super::minio::shared().await;
    let mut config = Config::contract_defaults();
    config.chunk_size = 65536;
    config.db_max_connections = 16;
    config.max_data_ops = 12;
    config.durability_profile = anchorfs_persistence::settings::DurabilityProfile::Development;
    config.deployment_id =
        DeploymentId::parse(format!("mount-{}", Uuid::new_v4().simple())).unwrap();
    config.s3 = super::minio::backend_config(
        &minio.endpoint,
        config.deployment_id.clone(),
        anchorfs_backend::config::ChecksumMode::Sha256,
        Duration::from_secs(10),
        0,
    );
    configure(&mut config);
    meta::initialize(
        &pool,
        &config.deployment_id,
        &DeploymentQuotaConfig {
            physical_bytes_limit: 1 << 40,
            staging_bytes_limit: 1 << 30,
        },
        config.chunk_size,
    )
    .await
    .unwrap();
    let issued = anchorfs_service::bootstrap::create_administrator(
        &pool,
        identity.uid,
        identity.gid,
        1 << 30,
        None,
    )
    .await
    .unwrap();
    let admin = Credential {
        id: issued.id.0,
        bearer: format!("{}.{}", issued.id, issued.secret.expose()),
    };
    let backend = Arc::new(anchorfs_backend::s3::S3Backend::from_config(&config.s3).unwrap());
    let mut state = AppState::production(config.clone(), pool.clone(), backend);
    state.faults = faults;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, build_router(state)).await.unwrap();
    });
    let created = raw(
        &endpoint,
        &admin,
        None,
        "POST",
        "/v1/workspaces",
        Some(json!({
            "root_uid": identity.uid, "root_gid": identity.gid, "root_mode": 493,
            "logical_bytes_limit": quota, "logical_inodes_limit": 100000,
            "physical_bytes_limit": 1u64 << 40, "staging_bytes_limit": 1u64 << 30,
        })),
    )
    .await;
    let workspace =
        Uuid::parse_str(created["workspace"]["workspace_id"].as_str().unwrap()).unwrap();
    let host = issue(
        &endpoint,
        &admin,
        workspace,
        "host",
        "write",
        identity.uid,
        identity.gid,
    )
    .await;
    let worker = issue(
        &endpoint,
        &host,
        workspace,
        "worker",
        "write",
        identity.uid,
        identity.gid,
    )
    .await;
    let sessions = format!("/v1/workspaces/{workspace}/sessions");
    let host_session = raw(
        &endpoint,
        &host,
        None,
        "POST",
        &sessions,
        Some(json!({"credential_id": host.id})),
    )
    .await;
    let session = raw(
        &endpoint,
        &host,
        None,
        "POST",
        &sessions,
        Some(json!({"credential_id": worker.id})),
    )
    .await;
    let api = native(&endpoint, workspace, &host, Some(&host_session));
    RunningService {
        ready: Ready {
            endpoint,
            pool,
            workspace,
            host,
            worker,
            admin,
            session,
            host_session,
            api,
            config,
            rt,
        },
        container,
        task,
    }
}
