mod common;
use anchorfs_client::{
    Client as NativeClient,
    auth::{Authority, Bearer, SessionAuthority},
    endpoint::Endpoint,
    retry::IdempotencyKey,
    wire::*,
};
use anchorfs_core::SessionId;
use common::{api_proxy::ApiProxy, fixture::*};
use std::sync::atomic::Ordering;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn native(f: &Fixture) -> (NativeClient, ApiProxy, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.client.app.clone();
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let proxy = ApiProxy::start(address).await;
    let authority = Authority {
        bearer: Bearer::parse(&format!("{}.{}", f.host.id, f.host.secret)).unwrap(),
        session: Some(SessionAuthority {
            session_id: SessionId(
                f.host_session["session_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
            ),
            session_epoch: f.host_session["session_epoch"].as_i64().unwrap(),
        }),
        workflow: None,
    };
    let client = NativeClient::new(
        Endpoint::parse(&proxy.endpoint, true).unwrap(),
        f.workspace,
        authority,
        None,
    )
    .unwrap();
    (client, proxy, server)
}
fn open(path: &str) -> OpenHandle {
    OpenHandle {
        path: path.into(),
        read: true,
        write: true,
        create: true,
        exclusive: true,
        truncate: false,
        mode: Some(0o644),
    }
}

#[tokio::test]
async fn client_resolves_lost_response_by_key_without_double_mutation() {
    let f = Fixture::with_bytes(
        workspace_body(),
        |c| {
            c.chunk_size = 4096;
        },
        Arc::new(anchorfs_service::pipeline::faults::NoFaults),
        false,
    )
    .await;
    let (client, proxy, _server) = native(&f).await;
    // Each call loses all three attempts, requiring the correct typed receipt resolver.
    let key = IdempotencyKey::new();
    proxy.drop_responses(3);
    let directory = client.create_directory("d", 0o755, &key).await.unwrap();
    assert_eq!(
        client.find_operation(&key).await.unwrap().operation_id,
        directory.operation_id
    );
    let before = proxy.count();
    assert_eq!(
        client
            .create_directory("different", 0o755, &key)
            .await
            .unwrap_err()
            .code(),
        Some("idempotency_mismatch")
    );
    assert_eq!(proxy.count(), before + 1);
    let key = IdempotencyKey::new();
    proxy.drop_responses(3);
    let handle = client.open_handle(&open("d/f"), &key).await.unwrap();
    assert_eq!(
        client.find_operation(&key).await.unwrap().operation_id,
        handle.operation_id
    );
    let key = IdempotencyKey::new();
    proxy.drop_responses(3);
    let write = client
        .write_at(handle.handle_id, 0, bytes::Bytes::from_static(b"abc"), &key)
        .await
        .unwrap();
    assert_eq!(write.bytes_written, 3);
    assert_eq!(
        client.find_operation(&key).await.unwrap().operation_id,
        write.operation_id
    );
    let key = IdempotencyKey::new();
    proxy.drop_responses(3);
    let append = client
        .append(handle.handle_id, bytes::Bytes::from_static(b"def"), &key)
        .await
        .unwrap();
    assert_eq!(append.offset, 3);
    assert_eq!(
        client.find_operation(&key).await.unwrap().operation_id,
        append.operation_id
    );
    assert_eq!(
        &client.read_range(handle.handle_id, 0, 6).await.unwrap().0[..],
        b"abcdef"
    );
    let key = IdempotencyKey::new();
    proxy.drop_responses(3);
    let session = client.create_session(f.worker.id, &key).await.unwrap();
    assert_eq!(
        client
            .find_admin_operation(&key)
            .await
            .unwrap()
            .operation_id,
        session.operation_id
    );
    assert_eq!(proxy.dropped.load(Ordering::SeqCst), 15);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND op_type IN ('directory_create','handle_open','write_at','append')").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 4);
    client
        .close_handle(handle.handle_id, &IdempotencyKey::new())
        .await
        .unwrap();
    f.invariants().await;
}

#[tokio::test]
async fn client_never_retries_indeterminate_or_4xx() {
    let f = Fixture::new().await;
    let (client, proxy, _server) = native(&f).await;
    let before = proxy.count();
    assert_eq!(
        client.stat(Some("missing")).await.unwrap_err().code(),
        Some("not_found")
    );
    assert_eq!(proxy.count(), before + 1);
    let key = IdempotencyKey::new();
    client.create_directory("d", 0o755, &key).await.unwrap();
    sqlx::query("UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND idempotency_key=$2").bind(f.workspace).bind(key.as_str()).execute(&f.pool).await.unwrap();
    let before = proxy.count();
    assert_eq!(
        client
            .create_directory("d", 0o755, &key)
            .await
            .unwrap_err()
            .code(),
        Some("operation_indeterminate")
    );
    assert_eq!(proxy.count(), before + 1);
    f.invariants().await;
}

#[tokio::test]
async fn client_wire_types_deserialize_service_dtos() {
    let f = Fixture::with_bytes(
        workspace_body(),
        |c| {
            c.chunk_size = 4096;
        },
        Arc::new(anchorfs_service::pipeline::faults::NoFaults),
        false,
    )
    .await;
    let prefix = f.workspace_path();
    macro_rules! decode_route {
        ($ty:ty, $method:expr, $suffix:expr, $key:expr, $body:expr) => {{
            let reply = f
                .client
                .call(
                    $method,
                    &format!("{prefix}{}", $suffix),
                    Some(&f.host),
                    Some(&f.host_session),
                    $key,
                    $body,
                )
                .await;
            assert!(reply.status.is_success(), "{}", reply.body);
            serde_json::from_value::<$ty>(reply.body).unwrap()
        }};
    }
    let workspace = decode_route!(WorkspaceInfo, "GET", "", None, None);
    assert!(workspace.workflow.is_none());
    let _: SessionGrant = serde_json::from_value(f.host_session.clone()).unwrap();
    let result = decode_route!(
        MutationResult,
        "POST",
        "/directories",
        Some("d"),
        Some(json!({"path":"d","mode":493}))
    );
    let _: InodeAttributes = decode_route!(InodeAttributes, "GET", "/stat?path=d", None, None);
    let page = decode_route!(DirectoryPage, "GET", "/directory", None, None);
    assert_eq!(page.entries[0].name, "d");
    let _: MutationResult = decode_route!(
        MutationResult,
        "POST",
        "/symlinks",
        Some("sym"),
        Some(json!({"path":"sym","target":"d"}))
    );
    let sym = decode_route!(SymlinkContents, "GET", "/symlinks?path=sym", None, None);
    assert_eq!(sym.target, "d");
    let h = decode_route!(
        HandleGrant,
        "POST",
        "/handles",
        Some("open"),
        Some(
            json!({"path":"d/f","read":true,"write":true,"create":true,"exclusive":true,"truncate":false,"mode":420})
        )
    );
    let reply = f
        .client
        .binary(
            "PUT",
            &format!("{prefix}/handles/{}/bytes?offset=0", h.handle_id),
            Some(&f.host),
            Some(&f.host_session),
            Some("write"),
            &[],
            axum::body::Body::from("abc"),
        )
        .await
        .status(200);
    let write: WriteResult = serde_json::from_slice(&reply.bytes).unwrap();
    assert_eq!(write.bytes_written, 3);
    let op = decode_route!(
        OperationStatus,
        "GET",
        format!("/operations/{}", result.operation_id),
        None,
        None
    );
    assert!(matches!(op.state, OperationState::Committed));
    let lease = f
        .client
        .call(
            "POST",
            &format!("{prefix}/workflow-lease"),
            Some(&f.host),
            Some(&f.host_session),
            Some("lease"),
            Some(json!({})),
        )
        .await
        .status(201);
    let workspace = decode_route!(WorkspaceInfo, "GET", "", None, None);
    assert!(workspace.workflow.unwrap().active);
    assert!(lease.body["token"].is_string());
    let _: MutationResult = decode_route!(
        MutationResult,
        "DELETE",
        format!("/handles/{}", h.handle_id),
        Some("close"),
        None
    );
    f.invariants().await;
}

#[tokio::test]
async fn client_resolves_deadline_after_publication() {
    let f = Fixture::new().await;
    let (mut client, proxy, _server) = native(&f).await;
    client.metadata_deadline = Duration::from_millis(500);
    let key = IdempotencyKey::new();
    proxy.delay_responses(3);
    let mutation = client.create_directory("late", 0o755, &key).await.unwrap();
    assert_eq!(
        client.find_operation(&key).await.unwrap().operation_id,
        mutation.operation_id
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key=$2",
    )
    .bind(f.workspace)
    .bind(key.as_str())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let attempts = {
        let requests = proxy.requests.lock().unwrap();
        requests
            .iter()
            .filter(|r| r.contains(&format!("idempotency-key: {}", key.as_str())))
            .count()
    };
    assert_eq!(attempts, 3);
    assert_eq!(
        client.stat(Some("late")).await.unwrap().inode_id,
        mutation.inode_id.unwrap()
    );
    f.invariants().await;
}
