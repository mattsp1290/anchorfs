const IMPLEMENTED: &[&str] = &[
    "acquireWorkflowLease",
    "renewWorkflowLease",
    "releaseWorkflowLease",
    "takeoverWorkflowLease",
    "createWorkspace",
    "getWorkspace",
    "createCredential",
    "rotateCredential",
    "revokeCredential",
    "createSession",
    "renewSession",
    "rebindSession",
    "revokeSession",
    "getAdminOperation",
    "findAdminOperation",
    "getOperation",
    "findOperation",
    "createDirectory",
    "removeDirectory",
    "unlink",
    "createLink",
    "createSymlink",
    "rename",
    "setAttributes",
    "statPath",
    "listDirectory",
    "readSymlink",
    "openHandle",
    "renewHandle",
    "closeHandle",
    "writeAt",
    "append",
    "truncate",
    "fsync",
    "readRange",
    "replaceFile",
    "createReadPin",
    "releaseReadPin",
];
mod common;
use anchorfs_persistence::settings::DurabilityProfile;
use anchorfs_service::{api_doc, app::build_router};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
async fn call(
    app: Router,
    method: &str,
    uri: &str,
    body: Vec<u8>,
    content_type: &str,
    key: bool,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", content_type)
        .header("content-length", body.len())
        .header("X-Anchorfs-Content-SHA256", "ab".repeat(32))
        .header(
            "X-Anchorfs-Session-Id",
            "00000000-0000-4000-8000-000000000001",
        )
        .header("X-Anchorfs-Session-Epoch", "1")
        .header("X-Anchorfs-Workflow-Token", "ab".repeat(32))
        .header("X-Anchorfs-Workflow-Epoch", "1");
    if key {
        builder = builder.header("Idempotency-Key", "synthetic-key");
    }
    let response = app
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let trace = response.headers()["X-Anchorfs-Trace-Id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    if value.get("code").is_some() {
        assert_eq!(value["trace_id"], trace);
    }
    (status, value)
}
fn fixture(op: &str) -> Value {
    let id = "00000000-0000-4000-8000-000000000001";
    match op {
        "createWorkspace" => {
            json!({"root_uid":1000,"root_gid":1000,"logical_bytes_limit":1024,"logical_inodes_limit":64,"physical_bytes_limit":1024,"staging_bytes_limit":512})
        }
        "createCredential" => {
            json!({"kind":"host","role":"write","uid":1000,"gid":1000,"workspace_id":id,"max_reservation_bytes":1024})
        }
        "createSession" => json!({"credential_id":id}),
        "rebindSession" => json!({}),
        "createDirectory" => json!({"path":"dir","mode":493}),
        "openHandle" => {
            json!({"path":"file","read":true,"write":true,"create":false,"exclusive":false,"truncate":false,"mode":420})
        }
        "truncate" => json!({"size":0}),
        "createReadPin" => json!({"path":"file"}),
        "createLink" => json!({"existing_path":"file","new_path":"other"}),
        "createSymlink" => json!({"path":"link","target":"file"}),
        "rename" => json!({"from":"file","to":"renamed"}),
        "setAttributes" => json!({"path":"file","mode":420}),
        _ => Value::Null,
    }
}
fn uri(path: &str, op: &Value) -> String {
    let mut uri = path.to_owned();
    while let Some(start) = uri.find('{') {
        let end = uri[start..].find('}').unwrap() + start;
        uri.replace_range(start..=end, "00000000-0000-4000-8000-000000000001");
    }
    let mut query = Vec::new();
    if let Some(params) = op["parameters"].as_array() {
        for p in params {
            if p["in"] == "query" && p["required"] == true {
                let name = p["name"].as_str().unwrap();
                let value = match name {
                    "path" => "file",
                    "idempotency_key" => "synthetic-key",
                    "length" => "1",
                    _ => "0",
                };
                query.push(format!("{name}={value}"));
            }
        }
    }
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(&query.join("&"));
    }
    uri
}
#[tokio::test]
async fn readyz_ok_on_default_postgres() {
    let (_container, pool, _) = common::database(&[]).await;
    let minio = common::minio::start().await;
    let (status, v) = call(
        build_router(common::state_with(pool, common::minio_s3(&minio))),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["backend"], "ok");
    assert_eq!(v["checksum_mode"], "sha256");
}
#[tokio::test]
async fn readyz_rejects_fsync_off() {
    let (_container, pool, _) = common::database(&["fsync=off"]).await;
    let minio = common::minio::start().await;
    let state = common::state_with(pool, common::minio_s3(&minio));
    let (status, v) = call(
        build_router(state.clone()),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(v["code"], "unsafe_database_settings");
    assert!(v["message"].as_str().unwrap().contains("fsync"));
    let mut config = (*state.config).clone();
    config.durability_profile = DurabilityProfile::Development;
    let state = anchorfs_service::app::AppState {
        cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
            .expect("validated cursor signing key"),
        config: Arc::new(config),
        ..state
    };
    let (status, v) = call(build_router(state), "GET", "/readyz", vec![], "", false).await;
    assert_eq!(status, 200);
    assert_eq!(v["durability_warnings"].as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn readyz_rejects_server_sync_commit_off() {
    let (_c, pool, _) = common::database(&["synchronous_commit=off"]).await;
    let (status, v) = call(
        build_router(common::state(pool)),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("synchronous_commit")
    );
    assert!(v["message"].as_str().unwrap().contains("server"));
}
#[tokio::test]
async fn readyz_schema_version_mismatch() {
    let (c, pool, _) = common::database(&[]).await;
    let mut update = c
        .exec(testcontainers::core::ExecCommand::new([
            "psql",
            "-U",
            "postgres",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "UPDATE schema_meta SET schema_version=2",
        ]))
        .await
        .unwrap();
    update.stdout_to_vec().await.unwrap();
    assert_eq!(update.exit_code().await.unwrap(), Some(0));
    let (status, v) = call(
        build_router(common::state(pool)),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(v["code"], "schema_version_mismatch");
}
#[tokio::test]
async fn readyz_without_database() {
    let start = std::time::Instant::now();
    let (status, v) = call(
        build_router(common::unavailable()),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(v["code"], "dependency_unavailable");
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
}
#[tokio::test]
async fn healthz_never_touches_database() {
    let (status, v) = call(
        build_router(common::unavailable()),
        "GET",
        "/healthz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(v["status"], "alive");
}
#[tokio::test]
async fn openapi_served() {
    let app = build_router(common::unavailable());
    let (status, v) = call(app.clone(), "GET", "/openapi.json", vec![], "", false).await;
    assert_eq!(status, 200);
    assert_eq!(v["info"]["version"], anchorfs_core::API_DOCUMENT_VERSION);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/openapi.yaml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(bytes, api_doc::document().to_yaml().unwrap());
}
#[tokio::test]
async fn implemented_routes_demand_bearer_and_others_return_501() {
    let app = build_router(common::unavailable());
    let doc: Value = serde_json::from_str(&api_doc::document().to_json().unwrap()).unwrap();
    let implemented = IMPLEMENTED;
    let mut implemented_count = 0;
    let mut count = 0;
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, op) in methods.as_object().unwrap() {
            let id = op["operationId"].as_str().unwrap();
            let binary = op["requestBody"]["content"]
                .get("application/octet-stream")
                .is_some();
            let body = if binary {
                vec![1, 2, 3]
            } else if op.get("requestBody").is_some() {
                serde_json::to_vec(&fixture(id)).unwrap()
            } else {
                vec![]
            };
            let (status, v) = call(
                app.clone(),
                &method.to_uppercase(),
                &uri(path, op),
                body,
                if binary {
                    "application/octet-stream"
                } else {
                    "application/json"
                },
                true,
            )
            .await;
            let implemented = implemented.contains(&id);
            assert_eq!(status, if implemented { 401 } else { 501 }, "{id}: {v}");
            assert_eq!(
                v["code"],
                if implemented {
                    "unauthenticated"
                } else {
                    "not_implemented"
                }
            );
            implemented_count += usize::from(implemented);
            count += 1;
        }
    }
    assert_eq!(count, 38);
    assert_eq!(implemented_count, 38);
}
#[tokio::test]
async fn invalid_body_returns_400_not_501() {
    let app = build_router(common::unavailable()).layer(axum::middleware::from_fn(
        async |mut req: axum::extract::Request, next: axum::middleware::Next| {
            req.headers_mut().insert(
                "Authorization",
                format!("Bearer {}.{}", uuid::Uuid::new_v4(), "a".repeat(64))
                    .parse()
                    .unwrap(),
            );
            next.run(req).await
        },
    ));
    let doc: Value = serde_json::from_str(&api_doc::document().to_json().unwrap()).unwrap();
    let mut json_count = 0;
    let mut key_count = 0;
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, op) in methods.as_object().unwrap() {
            let id = op["operationId"].as_str().unwrap();
            if op["requestBody"]["content"]
                .get("application/json")
                .is_some()
            {
                let (status, v) = call(
                    app.clone(),
                    &method.to_uppercase(),
                    &uri(path, op),
                    b"{".to_vec(),
                    "application/json",
                    true,
                )
                .await;
                assert_eq!(status, 400, "{id}");
                assert_eq!(v["code"], "invalid_input");
                json_count += 1;
            }
            if op["parameters"]
                .as_array()
                .is_some_and(|ps| ps.iter().any(|p| p["name"] == "Idempotency-Key"))
            {
                let (status, v) = call(
                    app.clone(),
                    &method.to_uppercase(),
                    &uri(path, op),
                    serde_json::to_vec(&fixture(id)).unwrap(),
                    "application/json",
                    false,
                )
                .await;
                assert_eq!(status, 400, "{id}");
                assert_eq!(v["code"], "invalid_input");
                key_count += 1;
            }
        }
    }
    assert_eq!(json_count, 12);
    assert!(key_count > 20);
}
#[tokio::test]
async fn body_limits_exact() {
    let state = common::unavailable();
    let write = state.config.max_request_bytes;
    let replace = state.config.max_replace_bytes;
    let app = build_router(state);
    let prefix = "/v1/workspaces/00000000-0000-4000-8000-000000000001";
    for (suffix, length, status) in [
        (
            "/handles/00000000-0000-4000-8000-000000000001/bytes?offset=0",
            write,
            401,
        ),
        (
            "/handles/00000000-0000-4000-8000-000000000001/bytes?offset=0",
            write + 1,
            413,
        ),
        ("/files?path=file&expected_generation=1", write + 1, 401),
        ("/files?path=file&expected_generation=1", replace + 1, 413),
    ] {
        let (s, v) = call(
            app.clone(),
            "PUT",
            &format!("{prefix}{suffix}"),
            vec![0; length],
            "application/octet-stream",
            true,
        )
        .await;
        assert_eq!(s, status, "{v}");
        if status == 413 {
            assert_eq!(v["code"], "payload_too_large");
        }
    }
}
#[tokio::test]
async fn unknown_route_and_method() {
    let app = build_router(common::unavailable());
    for (method, path, status, code) in [
        ("GET", "/v1/nope", 404, "not_found"),
        ("DELETE", "/healthz", 405, "method_not_allowed"),
    ] {
        let (s, v) = call(app.clone(), method, path, vec![], "", false).await;
        assert_eq!(s, status);
        assert_eq!(v["code"], code);
    }
}

#[tokio::test]
async fn root_selection_preserves_relative_path_rules() {
    let app = build_router(common::unavailable());
    let prefix = "/v1/workspaces/00000000-0000-4000-8000-000000000001";
    for suffix in [
        "/stat",
        "/directory",
        "/stat?path=file",
        "/directory?path=dir",
    ] {
        assert_eq!(
            call(
                app.clone(),
                "GET",
                &format!("{prefix}{suffix}"),
                vec![],
                "",
                true
            )
            .await
            .0,
            401
        );
    }
    assert_eq!(
        call(
            app.clone(),
            "PATCH",
            &format!("{prefix}/attributes"),
            br#"{"mode":493}"#.to_vec(),
            "application/json",
            true
        )
        .await
        .0,
        401
    );
    let app = app.layer(axum::middleware::from_fn(
        async |mut req: axum::extract::Request, next: axum::middleware::Next| {
            req.headers_mut().insert(
                "Authorization",
                format!("Bearer {}.{}", uuid::Uuid::new_v4(), "a".repeat(64))
                    .parse()
                    .unwrap(),
            );
            next.run(req).await
        },
    ));
    for path in ["", "%2F", "..", "a%2F..%2Fb", "%FF", "%ZZ"] {
        let (status, v) = call(
            app.clone(),
            "GET",
            &format!("{prefix}/stat?path={path}"),
            vec![],
            "",
            true,
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(v["code"], "invalid_input");
    }
}
#[tokio::test]
async fn session_and_workflow_wire_bindings_are_validated() {
    let app = build_router(common::unavailable());
    let doc: Value = serde_json::from_str(&api_doc::document().to_json().unwrap()).unwrap();
    let mut covered = 0;
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, op) in methods.as_object().unwrap() {
            let parameters = op["parameters"].as_array();
            if !parameters.is_some_and(|ps| ps.iter().any(|p| p["name"] == "X-Anchorfs-Session-Id"))
            {
                continue;
            }
            covered += 1;
            let id = op["operationId"].as_str().unwrap();
            let binary = op["requestBody"]["content"]
                .get("application/octet-stream")
                .is_some();
            let body = if binary {
                vec![0]
            } else if op.get("requestBody").is_some() {
                serde_json::to_vec(&fixture(id)).unwrap()
            } else {
                vec![]
            };
            for (session, epoch, token, workflow_epoch) in [
                (None, Some("1"), Some("ab".repeat(32)), Some("1")),
                (Some("bad-id"), Some("1"), None, None),
                (
                    Some("00000000-0000-4000-8000-000000000001"),
                    None,
                    None,
                    None,
                ),
                (
                    Some("00000000-0000-4000-8000-000000000001"),
                    Some("0"),
                    None,
                    None,
                ),
                (
                    Some("00000000-0000-4000-8000-000000000001"),
                    Some("1"),
                    Some("bad-token".into()),
                    Some("1"),
                ),
            ] {
                let mut req = Request::builder()
                    .method(method.to_uppercase().as_str())
                    .uri(uri(path, op))
                    .header(
                        "Authorization",
                        format!("Bearer {}.{}", uuid::Uuid::new_v4(), "a".repeat(64)),
                    )
                    .header("Idempotency-Key", "wire-test")
                    .header("Content-Length", body.len())
                    .header("X-Anchorfs-Content-SHA256", "ab".repeat(32))
                    .header(
                        "content-type",
                        if binary {
                            "application/octet-stream"
                        } else {
                            "application/json"
                        },
                    );
                if let Some(v) = session {
                    req = req.header("X-Anchorfs-Session-Id", v);
                }
                if let Some(v) = epoch {
                    req = req.header("X-Anchorfs-Session-Epoch", v);
                }
                if let Some(v) = token {
                    req = req.header("X-Anchorfs-Workflow-Token", v);
                }
                if let Some(v) = workflow_epoch {
                    req = req.header("X-Anchorfs-Workflow-Epoch", v);
                }
                let response = app
                    .clone()
                    .oneshot(req.body(Body::from(body.clone())).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), 400, "{id}");
                let value: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(value["code"], "invalid_input");
            }
            // Ordinary session authority needs no workflow proof; renew/release specifically require one.
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(uri(path, op))
                .header("Idempotency-Key", "wire-test")
                .header("Content-Length", body.len())
                .header("X-Anchorfs-Content-SHA256", "ab".repeat(32))
                .header(
                    "X-Anchorfs-Session-Id",
                    "00000000-0000-4000-8000-000000000001",
                )
                .header("X-Anchorfs-Session-Epoch", "1")
                .header(
                    "content-type",
                    if binary {
                        "application/octet-stream"
                    } else {
                        "application/json"
                    },
                )
                .body(Body::from(body))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                if ["renewWorkflowLease", "releaseWorkflowLease"].contains(&id) {
                    400
                } else if IMPLEMENTED.contains(&id) {
                    401
                } else {
                    501
                },
                "{id}"
            );
        }
    }
    assert_eq!(covered, 28);
}

#[tokio::test]
async fn readyz_backend_unreachable() {
    let (_db, pool, _) = common::database(&[]).await;
    let start = std::time::Instant::now();
    let (status, v) = call(
        build_router(common::state(pool)),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(v["code"], "dependency_unavailable");
    assert!(v["message"].as_str().unwrap().starts_with("backend:"));
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
}
#[tokio::test]
async fn readyz_backend_bucket_missing() {
    let (_db, pool, _) = common::database(&[]).await;
    let minio = common::minio::start().await;
    let mut config = common::minio_s3(&minio);
    config.bucket = "absent-bucket".into();
    let (status, v) = call(
        build_router(common::state_with(pool, config)),
        "GET",
        "/readyz",
        vec![],
        "",
        false,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(v["code"], "dependency_unavailable");
    assert!(v["message"].as_str().unwrap().contains("bucket not found"));
}
#[tokio::test]
async fn readyz_probe_result_is_memoized() {
    let (_db, pool, _) = common::database(&[]).await;
    let minio = common::minio::start().await;
    let proxy = common::proxy::Proxy::start(&minio.endpoint).await;
    let mut config = common::minio_s3(&minio);
    config.endpoint = proxy.endpoint.clone();
    let app = build_router(common::state_with(pool.clone(), config));
    proxy.script_for(
        common::proxy::is_list,
        [common::proxy::Fault::Delay(
            std::time::Duration::from_millis(100),
        )],
    );
    let (first, second) = tokio::join!(
        call(app.clone(), "GET", "/readyz", vec![], "", false),
        call(app.clone(), "GET", "/readyz", vec![], "", false),
    );
    assert_eq!(first.0, 200);
    assert_eq!(second.0, 200);
    assert_eq!(call(app, "GET", "/readyz", vec![], "", false).await.0, 200);
    assert_eq!(proxy.count(common::proxy::is_list), 1);
    // A separate scripted proxy isolates failure caching from the success window.
    let proxy = common::proxy::Proxy::start(&minio.endpoint).await;
    proxy.script_for(
        common::proxy::is_list,
        [
            common::proxy::Fault::Status {
                status: 503,
                code: "ServiceUnavailable",
            },
            common::proxy::Fault::Pass,
        ],
    );
    let mut config = common::minio_s3(&minio);
    config.endpoint = proxy.endpoint.clone();
    let state = common::state_with(pool, config);
    let app = build_router(state.clone());
    for _ in 0..2 {
        assert_eq!(
            call(app.clone(), "GET", "/readyz", vec![], "", false)
                .await
                .0,
            503
        );
    }
    assert_eq!(proxy.count(common::proxy::is_list), 1);
    // Age the memo directly so this test controls expiry without a wall-clock race.
    state.probe_cache.lock().await.as_mut().unwrap().0 =
        std::time::Instant::now() - std::time::Duration::from_secs(2);
    assert_eq!(call(app, "GET", "/readyz", vec![], "", false).await.0, 200);
    assert_eq!(proxy.count(common::proxy::is_list), 2);
}
#[tokio::test]
async fn readyz_backend_probe_deadline() {
    let (_db, pool, _) = common::database(&[]).await;
    let minio = common::minio::start().await;
    let proxy = common::proxy::Proxy::start(&minio.endpoint).await;
    proxy.script_for(
        common::proxy::is_list,
        [common::proxy::Fault::Delay(std::time::Duration::from_secs(
            3,
        ))],
    );
    let mut config = common::minio_s3(&minio);
    config.endpoint = proxy.endpoint.clone();
    let mut state = common::state_with(pool, config);
    Arc::make_mut(&mut state.config).backend_probe_deadline = std::time::Duration::from_millis(100);
    let start = std::time::Instant::now();
    let (status, v) = call(build_router(state), "GET", "/readyz", vec![], "", false).await;
    assert_eq!(status, 503);
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("backend: probe deadline exceeded")
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(proxy.count(common::proxy::is_list), 1);
}
