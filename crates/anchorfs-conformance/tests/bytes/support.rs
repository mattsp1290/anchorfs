pub use crate::common::client::RawReply;
pub use crate::common::fixture::*;
use crate::common::minio;
pub use anchorfs_core::OperationId;
pub use anchorfs_service::pipeline::faults::{FaultAction, FaultInjector, FaultPoint, NoFaults};
pub use axum::body::Body;

pub fn configure(config: &mut Config) {
    config.chunk_size = 4096;
    config.max_request_bytes = 4096;
}
pub async fn open(
    f: &Fixture,
    actor: &Credential,
    session: &Value,
    path: &str,
    create: bool,
) -> Value {
    f.client.call("POST",&format!("{}/handles",f.workspace_path()),Some(actor),Some(session),Some(&Uuid::new_v4().to_string()),
        Some(json!({"path":path,"read":true,"write":true,"create":create,"exclusive":false,"truncate":false,"mode":420})))
        .await.status(201).body
}
pub fn handle_path(f: &Fixture, handle: &Value) -> String {
    format!(
        "{}/handles/{}",
        f.workspace_path(),
        handle["handle_id"].as_str().unwrap()
    )
}
pub async fn write(
    f: &Fixture,
    handle: &Value,
    key: &str,
    offset: u64,
    bytes: impl Into<Vec<u8>>,
) -> RawReply {
    f.client
        .binary(
            "PUT",
            &format!("{}/bytes?offset={offset}", handle_path(f, handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            Some(key),
            &[],
            Body::from(bytes.into()),
        )
        .await
}
pub async fn append(f: &Fixture, handle: &Value, key: &str, bytes: impl Into<Vec<u8>>) -> RawReply {
    f.client
        .binary(
            "POST",
            &format!("{}/append", handle_path(f, handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            Some(key),
            &[],
            Body::from(bytes.into()),
        )
        .await
}
pub async fn read(
    f: &Fixture,
    handle: &Value,
    offset: u64,
    length: u32,
    pin: Option<Uuid>,
) -> RawReply {
    read_as(f, handle, &f.worker, &f.worker_session, offset, length, pin).await
}
pub async fn read_as(
    f: &Fixture,
    handle: &Value,
    actor: &Credential,
    session: &Value,
    offset: u64,
    length: u32,
    pin: Option<Uuid>,
) -> RawReply {
    let pin = pin.map(|id| format!("&read_pin={id}")).unwrap_or_default();
    f.client
        .binary(
            "GET",
            &format!(
                "{}/bytes?offset={offset}&length={length}{pin}",
                handle_path(f, handle)
            ),
            Some(actor),
            Some(session),
            None,
            &[],
            Body::empty(),
        )
        .await
}
pub async fn replace(
    f: &Fixture,
    path: &str,
    generation: i64,
    key: &str,
    bytes: impl Into<Vec<u8>>,
) -> RawReply {
    let bytes = bytes.into();
    let length = bytes.len().to_string();
    let digest = anchorfs_core::sha256(&bytes).to_string();
    f.client
        .binary(
            "PUT",
            &format!(
                "{}/files?path={path}&expected_generation={generation}",
                f.workspace_path()
            ),
            Some(&f.worker),
            Some(&f.worker_session),
            Some(key),
            &[
                ("Content-Length", &length),
                ("X-Anchorfs-Content-SHA256", &digest),
            ],
            Body::from(bytes),
        )
        .await
}
pub async fn pin(f: &Fixture, path: &str, key: &str) -> Value {
    f.client
        .call(
            "POST",
            &format!("{}/read-pins", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some(key),
            Some(json!({"path":path})),
        )
        .await
        .status(201)
        .body
}
pub fn error(reply: RawReply, status: u16, code: &str) -> Value {
    let reply = reply.status(status);
    let body = reply.json();
    assert_eq!(body["code"], code);
    assert_eq!(
        body["trace_id"],
        reply.headers["X-Anchorfs-Trace-Id"].to_str().unwrap()
    );
    body
}
/// Independent observation before readRange is connected: immutable manifest
/// rows plus raw SDK GETs, without using the service's chunk planners.
pub async fn committed(f: &Fixture, handle: &Value) -> Vec<u8> {
    let inode = handle["attributes"]["inode_id"].as_i64().unwrap();
    let (manifest,size,chunk_size):(Uuid,i64,i32)=sqlx::query_as("SELECT m.id,m.size,m.chunk_size FROM inodes i JOIN manifests m ON (m.workspace_id,m.id)=(i.workspace_id,i.current_manifest_id) WHERE i.workspace_id=$1 AND i.id=$2")
        .bind(f.workspace).bind(inode).fetch_one(&f.pool).await.unwrap();
    let rows:Vec<(i32,Uuid,Vec<u8>,i32,i64)>=sqlx::query_as("SELECT c.chunk_index,c.object_id,c.sha256,c.logical_length,o.byte_length FROM manifest_chunks c JOIN objects o ON (o.workspace_id,o.id)=(c.workspace_id,c.object_id) WHERE c.workspace_id=$1 AND c.manifest_id=$2 ORDER BY c.chunk_index")
        .bind(f.workspace).bind(manifest).fetch_all(&f.pool).await.unwrap();
    let Backend::Minio {
        deployment,
        endpoint,
        ..
    } = &f.backend
    else {
        panic!("byte fixture expected");
    };
    let client = minio::raw_client(endpoint).await;
    let mut bytes = vec![0; size as usize];
    for (index, id, digest, logical, length) in rows {
        let object = minio::raw_get(&client, &format!("{deployment}/{}/{id}", f.workspace))
            .await
            .expect("published object exists");
        assert_eq!(object.len(), length as usize);
        assert_eq!(anchorfs_core::sha256(&object).0.as_slice(), digest);
        let start = index as usize * chunk_size as usize;
        bytes[start..start + logical as usize].copy_from_slice(&object[..logical as usize]);
    }
    bytes
}
pub struct CrashOnce {
    pub point: FaultPoint,
    pub fired: std::sync::atomic::AtomicBool,
}
impl CrashOnce {
    pub fn at(point: FaultPoint) -> Self {
        Self {
            point,
            fired: false.into(),
        }
    }
}
impl FaultInjector for CrashOnce {
    fn at(&self, point: FaultPoint, _: Option<OperationId>) -> FaultAction {
        if point == self.point && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst) {
            FaultAction::Crash
        } else {
            FaultAction::Continue
        }
    }
}

pub struct GateFault {
    point: FaultPoint,
    limit: usize,
    arrivals: std::sync::atomic::AtomicUsize,
    gate: tokio::sync::watch::Receiver<bool>,
}
impl GateFault {
    pub fn new(point: FaultPoint, limit: usize) -> (Arc<Self>, tokio::sync::watch::Sender<bool>) {
        let (send, gate) = tokio::sync::watch::channel(false);
        (
            Arc::new(Self {
                point,
                limit,
                arrivals: 0.into(),
                gate,
            }),
            send,
        )
    }
    pub async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while self.arrivals.load(std::sync::atomic::Ordering::SeqCst) < self.limit {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("pipeline reached the controlled fault point");
    }
}
impl FaultInjector for GateFault {
    fn at(&self, point: FaultPoint, _: Option<OperationId>) -> FaultAction {
        if point == self.point
            && self
                .arrivals
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                < self.limit
        {
            FaultAction::Wait(self.gate.clone())
        } else {
            FaultAction::Continue
        }
    }
}
pub fn append_task(
    f: &Fixture,
    handle: &Value,
    key: &str,
    body: &[u8],
) -> tokio::task::JoinHandle<RawReply> {
    let client = Client {
        app: f.client.app.clone(),
    };
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!("{}/append", handle_path(f, handle));
    let key = key.to_string();
    let body = body.to_vec();
    tokio::spawn(async move {
        client
            .binary(
                "POST",
                &path,
                Some(&actor),
                Some(&session),
                Some(&key),
                &[],
                Body::from(body),
            )
            .await
    })
}
