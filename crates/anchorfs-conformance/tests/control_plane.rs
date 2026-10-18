//! HTTP acceptance from approved Stage 1a decisions, with disposable metadata only.
mod common;
use anchorfs_core::CredentialId;
use anchorfs_service::{
    app::{AppState, build_router},
    config::Config,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    client::{Client, Credential},
    fake_backend::FakeBackend,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

struct Fixture {
    _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    pool: PgPool,
    client: Client,
    backend: Arc<FakeBackend>,
    admin: Credential,
    host: Credential,
    worker: Credential,
    workspace: Uuid,
    host_session: Value,
    worker_session: Value,
}
fn workspace_body() -> Value {
    json!({"root_uid":1000,"root_gid":1000,"logical_bytes_limit":10000,"logical_inodes_limit":100,"physical_bytes_limit":20000,"staging_bytes_limit":10000})
}
fn credential_body(workspace: Uuid, kind: &str) -> Value {
    json!({"workspace_id":workspace,"kind":kind,"role":"write","uid":1000,"gid":1000,"max_reservation_bytes":4096})
}
impl Fixture {
    async fn new() -> Self {
        let (container, pool, _) = common::database(&[]).await;
        let (state, backend) = common::state_fake(pool.clone());
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
                Some(workspace_body()),
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
    fn client_with_deadline(&self, millis: u64) -> Client {
        let mut config = Config::contract_defaults();
        config.metadata_call_deadline = Duration::from_millis(millis);
        Client {
            app: build_router(AppState {
                pool: self.pool.clone(),
                config: Arc::new(config),
                backend: self.backend.clone(),
                probe_cache: Default::default(),
                control_permits: Arc::new(tokio::sync::Semaphore::new(2)),
            }),
        }
    }
    fn workspace_path(&self) -> String {
        format!("/v1/workspaces/{}", self.workspace)
    }
    fn session_path(&self, session: &Value, action: &str) -> String {
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

#[tokio::test]
async fn cp2_cp3_authentication_shapes_and_workspace_isolation() {
    let f = Fixture::new().await;
    let path = f.workspace_path();
    let root = f
        .client
        .call("GET", &path, Some(&f.worker), None, None, None)
        .await
        .status(200)
        .body["root"]
        .clone();
    assert_eq!(root["kind"], "directory");
    assert_eq!(root["mode"], 0o755);
    assert_eq!(root["uid"], 1000);
    assert_eq!(root["gid"], 1000);
    assert_eq!(root["nlink"], 2);
    assert_eq!(root["generation"], 1);
    for path in [
        path.clone(),
        format!("/v1/workspaces/{}", Uuid::new_v4()),
        format!("/v1/admin-operations/{}", Uuid::new_v4()),
    ] {
        let reply = f
            .client
            .call("GET", &path, None, None, None, None)
            .await
            .error(401, "unauthenticated");
        assert_eq!(reply.headers["WWW-Authenticate"], "Bearer");
        let wrong = Credential {
            id: f.admin.id,
            secret: "0".repeat(64),
        };
        let reply = f
            .client
            .call("GET", &path, Some(&wrong), None, None, None)
            .await
            .error(401, "unauthenticated");
        assert_eq!(
            reply.headers["WWW-Authenticate"],
            "Bearer error=\"invalid_token\""
        );
    }
    f.client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.worker),
            None,
            Some("forbidden"),
            Some(workspace_body()),
        )
        .await
        .error(403, "forbidden");
    let other = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("other-workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let other = format!(
        "/v1/workspaces/{}",
        other["workspace"]["workspace_id"].as_str().unwrap()
    );
    f.client
        .call("GET", &other, Some(&f.host), None, None, None)
        .await
        .error(404, "not_found");
    f.client
        .call("GET", &other, Some(&f.admin), None, None, None)
        .await
        .status(200);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/workspaces")
        .header("Authorization", "Basic invalid")
        .header("content-type", "application/json")
        .body(Body::from("{"))
        .unwrap();
    f.client.raw(request).await.error(401, "unauthenticated");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/workspaces")
        .header(
            "Authorization",
            format!("Bearer {}.{}", Uuid::new_v4(), "a".repeat(64)),
        )
        .header("Idempotency-Key", "malformed")
        .header("content-type", "application/json")
        .body(Body::from("{"))
        .unwrap();
    f.client.raw(request).await.error(400, "invalid_input");
    f.client
        .call("GET", "/v1/workspaces/bad", None, None, None, None)
        .await
        .error(400, "invalid_input");
    sqlx::query(
        "UPDATE credentials SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(f.worker.id)
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call("GET", &path, Some(&f.worker), None, None, None)
        .await
        .error(401, "unauthenticated");
}
#[tokio::test]
async fn cp4_cp12_receipts_replay_mismatch_visibility_and_tombstones() {
    let f = Fixture::new().await;
    let created = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    assert_eq!(
        created["workspace"]["workspace_id"],
        f.workspace.to_string()
    );
    let mut changed = workspace_body();
    changed["root_uid"] = json!(1001);
    let mismatch = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("workspace"),
            Some(changed),
        )
        .await
        .error(409, "idempotency_mismatch");
    assert_eq!(mismatch.body["operation_id"], created["operation_id"]);
    let replay = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("host"),
            Some(credential_body(f.workspace, "host")),
        )
        .await
        .status(201);
    assert!(replay.body["secret"].is_null());
    assert_eq!(replay.body["credential_id"], f.host.id.to_string());
    let receipt = f
        .client
        .call(
            "GET",
            "/v1/admin-operations?idempotency_key=host",
            Some(&f.admin),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert!(receipt["result"]["secret"].is_null());
    assert!(!receipt.to_string().contains(&f.host.secret));
    let foreign = format!(
        "/v1/admin-operations/{}",
        receipt["operation_id"].as_str().unwrap()
    );
    f.client
        .call("GET", &foreign, Some(&f.host), None, None, None)
        .await
        .error(404, "not_found");
    let renew_path = f.session_path(&f.worker_session, "/renew");
    let renewed = f
        .client
        .call(
            "POST",
            &renew_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew"),
            None,
        )
        .await
        .status(200)
        .body;
    let replay = f
        .client
        .call(
            "POST",
            &renew_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew"),
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(renewed, replay);
    let op_path = format!(
        "{}/operations/{}",
        f.workspace_path(),
        renewed["operation_id"].as_str().unwrap()
    );
    f.client
        .call("GET", &op_path, Some(&f.host), None, None, None)
        .await
        .status(200);
    f.client
        .call("GET", &op_path, Some(&f.worker), None, None, None)
        .await
        .status(200);
    f.client
        .call("GET", &op_path, Some(&f.admin), None, None, None)
        .await
        .error(404, "not_found");
    f.client
        .call(
            "GET",
            &format!("{}/operations?idempotency_key=renew", f.workspace_path()),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .status(200);
    sqlx::query(
        "UPDATE admin_operations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(Uuid::parse_str(created["operation_id"].as_str().unwrap()).unwrap())
    .execute(&f.pool)
    .await
    .unwrap();
    let expired = f
        .client
        .call(
            "GET",
            "/v1/admin-operations?idempotency_key=workspace",
            Some(&f.admin),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(expired["state"], "indeterminate");
    assert!(expired["result"].is_null());
    let tombstone: (String, Option<Value>) = sqlx::query_as(
        "SELECT state,result FROM admin_operations WHERE idempotency_key='workspace'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(tombstone, ("indeterminate".into(), None));
    let expired = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("workspace"),
            Some(workspace_body()),
        )
        .await
        .error(409, "operation_indeterminate");
    assert_eq!(expired.body["operation_id"], created["operation_id"]);
    sqlx::query(
        "UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(Uuid::parse_str(renewed["operation_id"].as_str().unwrap()).unwrap())
    .execute(&f.pool)
    .await
    .unwrap();
    let expired = f
        .client
        .call("GET", &op_path, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(expired["state"], "indeterminate");
    assert!(expired["result"].is_null());
    f.client
        .call(
            "POST",
            &renew_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew"),
            None,
        )
        .await
        .error(409, "operation_indeterminate");
}
#[tokio::test]
async fn cp6_credential_authority_and_secret_hygiene() {
    let f = Fixture::new().await;
    let mut body = credential_body(f.workspace, "worker");
    body["uid"] = json!(2000);
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("foreign-uid"),
            Some(body),
        )
        .await
        .error(403, "forbidden");
    for (field, value) in [
        ("gid", json!(2000)),
        ("max_reservation_bytes", json!(4097)),
        ("kind", json!("host")),
    ] {
        let mut body = credential_body(f.workspace, "worker");
        body[field] = value;
        f.client
            .call(
                "POST",
                "/v1/credentials",
                Some(&f.host),
                None,
                Some(field),
                Some(body),
            )
            .await
            .error(403, "forbidden");
    }
    let body = credential_body(Uuid::new_v4(), "worker");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("missing-workspace"),
            Some(body),
        )
        .await
        .error(404, "not_found");
    let mut body = credential_body(f.workspace, "worker");
    body["role"] = json!("admin");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("bad-role"),
            Some(body),
        )
        .await
        .error(400, "invalid_input");
    let mut body = credential_body(f.workspace, "worker");
    body["expires_at"] = json!("2000-01-01T00:00:00Z");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("past-expiry"),
            Some(body),
        )
        .await
        .error(400, "invalid_input");
    let response = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("fresh-worker"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .status(201);
    assert_eq!(response.headers["Cache-Control"], "no-store");
    let issued = Credential::from_grant(&response.body);
    let receipts: Vec<Value> =
        sqlx::query_scalar("SELECT result FROM admin_operations WHERE result IS NOT NULL")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    for result in receipts {
        assert!(!result.to_string().contains(&issued.secret));
        assert!(!result.to_string().contains(&f.worker.secret));
    }
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.worker),
            None,
            Some("worker-issue"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .error(403, "forbidden");
}

#[tokio::test]
async fn cp7_cp8_cp11_session_authority_epochs_and_self_fencing() {
    let f = Fixture::new().await;
    let mint = format!("{}/sessions", f.workspace_path());
    let target = json!({"credential_id":f.worker.id});
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.worker),
            None,
            Some("worker-mint"),
            Some(target.clone()),
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.admin),
            None,
            Some("admin-mint"),
            Some(target.clone()),
        )
        .await
        .status(201);
    let mut lease = target.clone();
    lease["bind_to_workflow_lease"] = json!("a".repeat(64));
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("lease"),
            Some(lease),
        )
        .await
        .error(501, "not_implemented");
    let granted = f
        .client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("mint"),
            Some(target),
        )
        .await
        .status(201)
        .body;
    let now: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let expiry = time::OffsetDateTime::parse(
        granted["expires_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!((expiry - now).whole_seconds() >= 3590 && (expiry - now).whole_seconds() <= 3601);
    let renew = f.session_path(&f.worker_session, "/renew");
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.host_session),
            Some("foreign-session"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let mut stale = f.worker_session.clone();
    stale["session_epoch"] = json!(2);
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&stale),
            Some("bad-epoch"),
            None,
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &f.session_path(&f.host_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew-other"),
            None,
        )
        .await
        .error(403, "forbidden");
    let rebind = f.session_path(&f.worker_session, "/rebind");
    f.client
        .call(
            "POST",
            &rebind,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("worker-rebind"),
            Some(json!({})),
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("lease-rebind"),
            Some(json!({"workflow_lease_token":"a".repeat(64)})),
        )
        .await
        .error(501, "not_implemented");
    let rebound = f
        .client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(rebound["session_epoch"], 2);
    assert!(rebound["bound_mutation_epoch"].is_null());
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("old-epoch"),
            None,
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&rebound),
            Some("new-epoch"),
            None,
        )
        .await
        .status(200);
    sqlx::query(
        "UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(Uuid::parse_str(rebound["session_id"].as_str().unwrap()).unwrap())
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-renew"),
            None,
        )
        .await
        .error(409, "fenced");
    let restored = f
        .client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(restored["session_epoch"], 3);
    let revoke = f.session_path(&restored, "");
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.worker),
            Some(&restored),
            Some("self-revoke"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.worker),
            Some(&restored),
            Some("self-revoke"),
            None,
        )
        .await
        .error(409, "fenced");
    let host_rebind = f.session_path(&f.host_session, "/rebind");
    let host_epoch2 = f
        .client
        .call(
            "POST",
            &host_rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("self-rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    f.client
        .call(
            "POST",
            &host_rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("self-rebind"),
            Some(json!({})),
        )
        .await
        .error(409, "fenced");
    assert_eq!(host_epoch2["session_epoch"], 2);
    // Failed business mutations leave no receipt.
    let rejected: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE idempotency_key=ANY($1)")
            .bind(vec!["renew-other", "worker-rebind", "expired-renew"])
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(rejected, 0);
}
#[tokio::test]
async fn cp9_rotation_revokes_sessions_and_lost_self_rotation_is_recoverable() {
    let f = Fixture::new().await;
    let mint = format!("{}/sessions", f.workspace_path());
    for slot in 1..64 {
        f.client
            .call(
                "POST",
                &mint,
                Some(&f.host),
                None,
                Some(&format!("fill-{slot}")),
                Some(json!({"credential_id":f.worker.id})),
            )
            .await
            .status(201);
    }
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("over-limit"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .error(507, "quota_exceeded");
    let rotate = format!("/v1/credentials/{}/rotate", f.worker.id);
    f.client
        .call(
            "POST",
            &rotate,
            Some(&f.worker),
            None,
            Some("worker-rotate"),
            None,
        )
        .await
        .error(403, "forbidden");
    let issued = f
        .client
        .call(
            "POST",
            &rotate,
            Some(&f.host),
            None,
            Some("rotate-worker"),
            None,
        )
        .await
        .status(200);
    assert_eq!(issued.headers["Cache-Control"], "no-store");
    assert_eq!(issued.body["credential_epoch"], 2);
    let revoked_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NOT NULL",
    )
    .bind(f.worker.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(revoked_sessions, 64);
    let new_worker = Credential::from_grant(&issued.body);
    f.client
        .call(
            "GET",
            &f.workspace_path(),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .error(401, "unauthenticated");
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&new_worker),
            Some(&f.worker_session),
            Some("fenced-session"),
            None,
        )
        .await
        .error(409, "fenced");
    let replay = f
        .client
        .call(
            "POST",
            &rotate,
            Some(&f.host),
            None,
            Some("rotate-worker"),
            None,
        )
        .await
        .status(200)
        .body;
    assert!(replay["secret"].is_null());
    assert_eq!(replay["credential_epoch"], 2);
    let session = f
        .client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.host),
            None,
            Some("post-rotation-mint"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .status(201)
        .body;
    assert_eq!(session["credential_epoch"], 2);
    let self_rotate = format!("/v1/credentials/{}/rotate", f.host.id);
    // Deliberately discard the secret response to model the lost first response.
    f.client
        .call(
            "POST",
            &self_rotate,
            Some(&f.host),
            None,
            Some("self-rotate"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &self_rotate,
            Some(&f.host),
            None,
            Some("self-rotate"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let recovered = f
        .client
        .call(
            "POST",
            &self_rotate,
            Some(&f.admin),
            None,
            Some("recover-host"),
            None,
        )
        .await
        .status(200)
        .body;
    let recovered = Credential::from_grant(&recovered);
    f.client
        .call(
            "GET",
            &f.workspace_path(),
            Some(&recovered),
            None,
            None,
            None,
        )
        .await
        .status(200);
    let stale_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL",
    )
    .bind(f.host.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(stale_count, 0);
}
#[tokio::test]
async fn cp10_revocation_cascades_and_bootstrap_recovers_all_administrators_revoked() {
    let f = Fixture::new().await;
    let revoke = format!("/v1/credentials/{}", f.host.id);
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.host),
            None,
            Some("self-revoke-host"),
            None,
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.admin),
            None,
            Some("revoke-host"),
            None,
        )
        .await
        .status(200);
    for credential in [&f.host, &f.worker] {
        f.client
            .call(
                "GET",
                &f.workspace_path(),
                Some(credential),
                None,
                None,
                None,
            )
            .await
            .error(401, "unauthenticated");
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM credentials WHERE id=ANY($1) AND revoked_at IS NOT NULL AND epoch=2",
    )
    .bind(vec![f.host.id, f.worker.id])
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 2);
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM client_sessions WHERE id=$1")
            .bind(Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(revoked);
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.admin),
            None,
            Some("revoked-mint"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .error(404, "not_found");
    let admin_b=f.client.call("POST","/v1/credentials",Some(&f.admin),None,Some("admin-b"),Some(json!({"kind":"administrator","role":"admin","uid":1000,"gid":1000,"max_reservation_bytes":4096}))).await.status(201).body;
    let b = Credential::from_grant(&admin_b);
    f.client
        .call(
            "DELETE",
            &format!("/v1/credentials/{}", f.admin.id),
            Some(&b),
            None,
            Some("revoke-ancestry"),
            None,
        )
        .await
        .status(200);
    for credential in [&f.admin, &b] {
        f.client
            .call(
                "GET",
                &f.workspace_path(),
                Some(credential),
                None,
                None,
                None,
            )
            .await
            .error(401, "unauthenticated");
    }
    anchorfs_service::bootstrap::create_administrator(&f.pool, 1000, 1000, 4096, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn cp5_same_key_http_smoke_and_pending_owner_budget() {
    use anchorfs_persistence::receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope};
    let f = Fixture::new().await;
    let first = f.client.call(
        "POST",
        "/v1/workspaces",
        Some(&f.admin),
        None,
        Some("concurrent"),
        Some(workspace_body()),
    );
    let second = f.client.call(
        "POST",
        "/v1/workspaces",
        Some(&f.admin),
        None,
        Some("concurrent"),
        Some(workspace_body()),
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.status(201).body, second.status(201).body);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut owner = f.pool.begin().await.unwrap();
    let scope = ReceiptScope::Admin {
        credential: CredentialId(f.admin.id),
    };
    let digest =
        anchorfs_service::receipt::request_digest("workspace_create", &[], Some(&workspace_body()));
    let entry = receipts::begin(
        &mut owner,
        scope,
        ReceiptRequest {
            key: "pending",
            op_type: "workspace_create",
            digest,
            credential_epoch: 1,
            retention: Duration::from_secs(3600),
            wait_bound: Duration::from_secs(1),
        },
    )
    .await
    .unwrap();
    let ReceiptEntry::Fresh(owner_id) = entry else {
        panic!("fresh owner expected");
    };
    let client = f.client_with_deadline(800);
    let started = std::time::Instant::now();
    client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("pending"),
            Some(workspace_body()),
        )
        .await
        .error(503, "outcome_unknown");
    assert!(started.elapsed() < Duration::from_millis(1500));
    // The pending owner's key is still private. The waiter cannot publish a second row.
    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM admin_operations WHERE idempotency_key='pending'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(visible, 0);
    // Drain detached work before reading its final state; timeout/rollback cannot publish.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.pool.num_idle() + 1 >= f.pool.size() as usize {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    receipts::commit_result(
        &mut owner,
        scope,
        owner_id,
        &json!({"operation_id":owner_id.0,"synthetic_owner":true}),
    )
    .await
    .unwrap();
    owner.commit().await.unwrap();
    let committed = f
        .client
        .call(
            "GET",
            "/v1/admin-operations?idempotency_key=pending",
            Some(&f.admin),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(committed["state"], "committed");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}
#[tokio::test]
async fn cp13a_abandoned_request_still_commits_and_resolves_by_key() {
    let f = Fixture::new().await;
    let client = f.client_with_deadline(400);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM credentials WHERE id=$1 FOR UPDATE")
        .bind(f.admin.id)
        .execute(&mut *holder)
        .await
        .unwrap();
    let release = async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        holder.commit().await.unwrap();
    };
    let request = async {
        let started = std::time::Instant::now();
        client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&f.admin),
                None,
                Some("abandoned"),
                Some(workspace_body()),
            )
            .await
            .error(503, "outcome_unknown");
        assert!(started.elapsed() < Duration::from_secs(1));
    };
    tokio::join!(release, request);
    let receipt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = f
                .client
                .call(
                    "GET",
                    "/v1/admin-operations?idempotency_key=abandoned",
                    Some(&f.admin),
                    None,
                    None,
                    None,
                )
                .await;
            if result.status == StatusCode::OK {
                break result.body;
            }
            assert_eq!(result.status, StatusCode::NOT_FOUND);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(receipt["state"], "committed");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(after, before + 1);
}
#[tokio::test]
async fn cp13b_commit_connection_loss_reports_unknown_outcome() {
    let f = Fixture::new().await;
    // A deferred constraint trigger is test-only coordination at the real COMMIT.
    // It is installed solely in this disposable database, never in migrations.
    sqlx::raw_sql("CREATE FUNCTION test_commit_barrier() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(918271); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER test_commit_barrier AFTER INSERT ON workspaces DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_commit_barrier();").execute(&f.pool).await.unwrap();
    let mut holder = f.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(918271)")
        .execute(&mut *holder)
        .await
        .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let client = Client {
        app: f.client.app.clone(),
    };
    let admin = f.admin.clone();
    let request = tokio::spawn(async move {
        client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&admin),
                None,
                Some("commit-loss"),
                Some(workspace_body()),
            )
            .await
    });
    let pid:i32=tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let pid:Option<i32>=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND query='COMMIT' AND wait_event_type='Lock' AND wait_event='advisory'").fetch_optional(&f.pool).await.unwrap();
            if let Some(pid)=pid {break pid;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(terminated);
    request.await.unwrap().error(503, "outcome_unknown");
    sqlx::query("SELECT pg_advisory_unlock(918271)")
        .execute(&mut *holder)
        .await
        .unwrap();
    drop(holder);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_operations WHERE idempotency_key='commit-loss'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!((0..=1).contains(&(after - before)));
    assert_eq!(after - before, receipts);
}

#[tokio::test]
async fn cp2_cp3_cp6_cp8_matrix_edges() {
    let f = Fixture::new().await;
    for invalid in [
        "Bearer x".to_string(),
        format!("Bearer {}.{}", f.admin.id, "a".repeat(63)),
        format!("Bearer {}.{}", Uuid::new_v4(), "a".repeat(64)),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/workspaces")
            .header("Authorization", invalid)
            .header("Idempotency-Key", "bad-bearer")
            .header("content-type", "application/json")
            .body(Body::from(workspace_body().to_string()))
            .unwrap();
        let reply = f.client.raw(request).await.error(401, "unauthenticated");
        assert!(!reply.body.to_string().contains(&f.admin.secret));
    }
    let wrong = Credential {
        id: f.admin.id,
        secret: "0".repeat(64),
    };
    f.client
        .call(
            "POST",
            &format!("/v1/credentials/{}/rotate", Uuid::new_v4()),
            Some(&wrong),
            None,
            Some("unknown-target"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    for (field, value) in [
        ("root_mode", json!(2541)),
        ("logical_bytes_limit", json!(0)),
    ] {
        let mut body = workspace_body();
        body[field] = value;
        f.client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&f.admin),
                None,
                Some(field),
                Some(body),
            )
            .await
            .error(400, "invalid_input");
    }
    let missing_session = json!({"session_id":Uuid::new_v4(),"session_epoch":1});
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&missing_session),
            Some("unknown-session"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let admin_owned = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("admin-owned"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .status(201)
        .body;
    let worker = Credential::from_grant(&admin_owned);
    f.client
        .call(
            "POST",
            &format!("/v1/credentials/{}/rotate", worker.id),
            Some(&f.host),
            None,
            Some("not-issuer"),
            None,
        )
        .await
        .error(403, "forbidden");
    let renewed = f
        .client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("private-receipt"),
            None,
        )
        .await
        .status(200)
        .body;
    f.client
        .call(
            "GET",
            &format!(
                "{}/operations/{}",
                f.workspace_path(),
                renewed["operation_id"].as_str().unwrap()
            ),
            Some(&worker),
            None,
            None,
            None,
        )
        .await
        .error(404, "not_found");
    f.client
        .call(
            "DELETE",
            &f.session_path(&f.worker_session, ""),
            Some(&f.host),
            Some(&f.host_session),
            Some("host-revoke-session"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("revoked-actor"),
            None,
        )
        .await
        .error(409, "fenced");
    let other = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("other"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let other = Uuid::parse_str(other["workspace"]["workspace_id"].as_str().unwrap()).unwrap();
    let other_host = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("other-host"),
            Some(credential_body(other, "host")),
        )
        .await
        .status(201)
        .body;
    let mint = format!("{}/sessions", f.workspace_path());
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("foreign-target"),
            Some(json!({"credential_id":other_host["credential_id"]})),
        )
        .await
        .error(404, "not_found");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("foreign-workspace"),
            Some(credential_body(other, "worker")),
        )
        .await
        .error(404, "not_found");
    let mut null_workspace = credential_body(f.workspace, "worker");
    null_workspace["workspace_id"] = Value::Null;
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("null-workspace"),
            Some(null_workspace),
        )
        .await
        .error(400, "invalid_input");
    sqlx::query(
        "UPDATE credentials SET expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1",
    )
    .bind(f.host.id)
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("unbounded-lifetime"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .error(403, "forbidden");
    let mut bounded = credential_body(f.workspace, "worker");
    let expiry: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM credentials WHERE id=$1")
            .bind(f.host.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    bounded["expires_at"] = json!(anchorfs_service::control::timestamp(expiry));
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("bounded-lifetime"),
            Some(bounded),
        )
        .await
        .status(201);
}

#[tokio::test]
async fn retryable_database_states_have_bounded_retry_guidance() {
    let (_container, pool, _) = common::database(&[]).await;
    for code in ["40P01", "40001", "55P03", "57014"] {
        // Only these four fixed synthetic SQLSTATEs enter the test statement.
        let statement = format!("DO $$ BEGIN RAISE SQLSTATE '{code}'; END $$");
        let error = sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap_err();
        let mapped = anchorfs_service::error::database(error, "synthetic");
        assert_eq!(mapped.code, anchorfs_core::ErrorCode::DependencyUnavailable);
        assert_eq!(mapped.retry_after_ms, Some(100));
    }
}

#[tokio::test]
async fn expired_rebind_respects_capacity_and_active_rebind_keeps_its_slot() {
    let f = Fixture::new().await;
    let expired = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expired)
    .execute(&f.pool)
    .await
    .unwrap();
    let mint = format!("{}/sessions", f.workspace_path());
    let mut active = Value::Null;
    for slot in 0..64 {
        active = f
            .client
            .call(
                "POST",
                &mint,
                Some(&f.host),
                None,
                Some(&format!("replacement-{slot}")),
                Some(json!({"credential_id":f.worker.id})),
            )
            .await
            .status(201)
            .body;
    }
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-over-capacity"),
            Some(json!({})),
        )
        .await
        .error(507, "quota_exceeded");
    f.client
        .call(
            "POST",
            &f.session_path(&active, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some("active-at-capacity"),
            Some(json!({})),
        )
        .await
        .status(200);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp()").bind(f.worker.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 64);
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE idempotency_key='expired-over-capacity'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0);
}
async fn wait_for_lock(pool: &PgPool, query_fragment: &str) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND state='active' AND wait_event_type='Lock' AND position($1 in query)>0)").bind(query_fragment).fetch_one(pool).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}
async fn wait_until_expired(pool: &PgPool, expiry: time::OffsetDateTime) {
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
#[tokio::test]
async fn expired_credential_after_receipt_wait_cannot_publish_any_admin_mutation() {
    let f = Fixture::new().await;
    for operation in ["workspace", "credential", "session"] {
        let before:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM workspaces),(SELECT count(*) FROM credentials),(SELECT count(*) FROM client_sessions)").fetch_one(&f.pool).await.unwrap();
        let expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE credentials SET expires_at=clock_timestamp()+interval '1500 milliseconds' WHERE id=$1 RETURNING expires_at").bind(f.admin.id).fetch_one(&f.pool).await.unwrap();
        let mut holder = f.pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE admin_operations IN SHARE MODE")
            .execute(&mut *holder)
            .await
            .unwrap();
        let (path, body) = match operation {
            "workspace" => ("/v1/workspaces".to_string(), workspace_body()),
            "credential" => (
                "/v1/credentials".to_string(),
                json!({"kind":"administrator","role":"admin","uid":1000,"gid":1000,"max_reservation_bytes":4096}),
            ),
            _ => (
                format!("{}/sessions", f.workspace_path()),
                json!({"credential_id":f.worker.id}),
            ),
        };
        let client = Client {
            app: f.client.app.clone(),
        };
        let admin = f.admin.clone();
        let key = format!("late-{operation}");
        let request_key = key.clone();
        let request = tokio::spawn(async move {
            client
                .call(
                    "POST",
                    &path,
                    Some(&admin),
                    None,
                    Some(&request_key),
                    Some(body),
                )
                .await
        });
        wait_for_lock(&f.pool, "INSERT INTO admin_operations").await;
        wait_until_expired(&f.pool, expiry).await;
        holder.commit().await.unwrap();
        request.await.unwrap().error(401, "unauthenticated");
        let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM workspaces),(SELECT count(*) FROM credentials),(SELECT count(*) FROM client_sessions)").fetch_one(&f.pool).await.unwrap();
        assert_eq!(before, after);
        let receipts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM admin_operations WHERE idempotency_key=$1")
                .bind(key)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(receipts, 0);
        sqlx::query("UPDATE credentials SET expires_at=NULL WHERE id=$1")
            .bind(f.admin.id)
            .execute(&f.pool)
            .await
            .unwrap();
    }
}
#[tokio::test]
async fn expired_actor_session_after_target_wait_rolls_back_mutation_and_receipt() {
    let f = Fixture::new().await;
    let target = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    let actor = Uuid::parse_str(f.host_session["session_id"].as_str().unwrap()).unwrap();
    let before: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM client_sessions WHERE id=$1")
            .bind(target)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE client_sessions SET expires_at=clock_timestamp()+interval '1500 milliseconds' WHERE id=$1 RETURNING expires_at").bind(actor).fetch_one(&f.pool).await.unwrap();
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM client_sessions WHERE id=$1 FOR UPDATE")
        .bind(target)
        .execute(&mut *holder)
        .await
        .unwrap();
    let client = Client {
        app: f.client.app.clone(),
    };
    let host = f.host.clone();
    let session = f.host_session.clone();
    let path = f.session_path(&f.worker_session, "/renew");
    let request = tokio::spawn(async move {
        client
            .call(
                "POST",
                &path,
                Some(&host),
                Some(&session),
                Some("late-session"),
                None,
            )
            .await
    });
    wait_for_lock(&f.pool, "SELECT * FROM client_sessions").await;
    wait_until_expired(&f.pool, expiry).await;
    holder.commit().await.unwrap();
    request.await.unwrap().error(409, "fenced");
    let after: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM client_sessions WHERE id=$1")
            .bind(target)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE idempotency_key='late-session'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(receipts, 0);
}
