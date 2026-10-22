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
    let (client, proxy, _server) = native(&f).await;
    let client = client.with_deadlines(Duration::from_millis(500), Duration::from_secs(60));
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

#[tokio::test]
async fn client_keeps_ambiguity_when_replay_is_gated() {
    for gated_resolver in [false, true] {
        let f = Fixture::new().await;
        let (client, proxy, _server) = native(&f).await;
        let key = IdempotencyKey::new();
        let held = proxy.hold_response("POST /v1/workspaces/");
        proxy.drop_responses(1);
        let task = tokio::spawn({
            let client = client.clone();
            let key = key.clone();
            async move { client.create_directory("committed", 0o755, &key).await }
        });
        tokio::time::timeout(Duration::from_secs(10), held.reached())
            .await
            .unwrap();
        f.client
            .call(
                "POST",
                &format!("{}/workflow-lease", f.workspace_path()),
                Some(&f.host),
                Some(&f.host_session),
                Some("gate-retry"),
                Some(json!({})),
            )
            .await
            .status(201);
        if gated_resolver {
            proxy.fail_requests("/operations?", 1, 401, "unauthenticated");
        }
        held.release();
        let result = task.await.unwrap();
        if gated_resolver {
            assert_eq!(result.unwrap_err().code(), Some("outcome_unknown"));
        } else {
            let result = result.unwrap();
            assert_eq!(
                client.find_operation(&key).await.unwrap().operation_id,
                result.operation_id
            );
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key=$2 AND state='committed'")
            .bind(f.workspace).bind(key.as_str()).fetch_one(&f.pool).await.unwrap();
        assert_eq!(count, 1);
        assert_eq!(proxy.dropped.load(Ordering::SeqCst), 1);
        f.invariants().await;
    }
}

#[tokio::test]
async fn https_endpoint_verifies_server_certificate() {
    let pki = common::tls::generate();
    let tls = anchorfs_service::tls::server_config(&pki.cert_pem, &pki.key_pem).unwrap();
    let fixture_tls = tls.clone();
    let f = Fixture::with_bytes(
        workspace_body(),
        move |c| {
            c.chunk_size = 4096;
            c.tls = Some(fixture_tls);
        },
        Arc::new(anchorfs_service::pipeline::faults::NoFaults),
        false,
    )
    .await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = Endpoint::parse(
        &format!("https://{}", listener.local_addr().unwrap()),
        false,
    )
    .unwrap();
    let (handle, future) = anchorfs_service::tls::serve(
        listener,
        f.client.app.clone(),
        Some(tls),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    struct TlsServer(
        anchorfs_service::tls::ServerHandle,
        tokio::task::JoinHandle<std::io::Result<()>>,
    );
    impl Drop for TlsServer {
        fn drop(&mut self) {
            self.0.shutdown();
            self.1.abort();
        }
    }
    let server = TlsServer(handle, tokio::spawn(future));
    server.0.listening().await.unwrap();
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
        endpoint.clone(),
        f.workspace,
        authority.clone(),
        Some(&pki.ca_pem),
    )
    .unwrap();
    client.stat(None).await.unwrap();
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM operations) + (SELECT count(*) FROM admin_operations), (SELECT count(*) FROM client_sessions)",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let wrong =
        NativeClient::new(endpoint, f.workspace, authority, Some(&pki.other_ca_pem)).unwrap();
    assert!(matches!(
        wrong.stat(None).await.unwrap_err(),
        anchorfs_client::error::ClientError::Transport(_)
    ));
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM operations) + (SELECT count(*) FROM admin_operations), (SELECT count(*) FROM client_sessions)",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    client.stat(None).await.unwrap();
    let session = client
        .create_session(f.worker.id, &IdempotencyKey::new())
        .await
        .unwrap();
    assert_ne!(session.session_id, uuid::Uuid::nil());
    client
        .create_directory("tls", 0o755, &IdempotencyKey::new())
        .await
        .unwrap();
    let h = client
        .open_handle(&open("tls/file"), &IdempotencyKey::new())
        .await
        .unwrap();
    client
        .write_at(
            h.handle_id,
            0,
            bytes::Bytes::from_static(b"verified https"),
            &IdempotencyKey::new(),
        )
        .await
        .unwrap();
    client.fsync(h.handle_id).await.unwrap();
    assert_eq!(
        &client.read_range(h.handle_id, 0, 14).await.unwrap().0[..],
        b"verified https"
    );
    client
        .close_handle(h.handle_id, &IdempotencyKey::new())
        .await
        .unwrap();
    let mutations: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND op_type IN ('directory_create','handle_open','write_at','handle_close')")
        .bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(mutations, 4);
    let totals: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM operations) + (SELECT count(*) FROM admin_operations), (SELECT count(*) FROM client_sessions)",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    // TLS adds one session receipt and four byte/namespace mutation receipts.
    assert_eq!(totals, (before.0 + 5, before.1 + 1));
    f.invariants().await;
}
