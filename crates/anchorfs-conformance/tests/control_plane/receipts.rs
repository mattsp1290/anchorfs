use super::support::*;

#[tokio::test]
async fn expired_receipt_followup_recovers_after_a_crash() {
    use anchorfs_service::pipeline::faults::{FaultAction, FaultInjector, FaultPoint};
    use std::sync::atomic::{AtomicBool, Ordering};
    struct CrashOnce(AtomicBool);
    impl FaultInjector for CrashOnce {
        fn at(&self, point: FaultPoint, _: Option<anchorfs_core::OperationId>) -> FaultAction {
            if point == FaultPoint::BeforeFollowUp && !self.0.swap(true, Ordering::SeqCst) {
                FaultAction::Crash
            } else {
                FaultAction::Continue
            }
        }
    }
    let f = Fixture::new().await;
    let renew = f.session_path(&f.worker_session, "/renew");
    let renewed = f
        .client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("crash-before-followup"),
            None,
        )
        .await
        .status(200)
        .body;
    let operation = Uuid::parse_str(renewed["operation_id"].as_str().unwrap()).unwrap();
    let session = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=$2")
        .bind(f.workspace).bind(operation).execute(&f.pool).await.unwrap();
    let mut state = common::state_fake(f.pool.clone()).0;
    state.backend = f.backend.service();
    state.faults = Arc::new(CrashOnce(AtomicBool::new(false)));
    let crash_client = Client {
        app: build_router(state),
    };
    let path = format!("{}/operations/{operation}", f.workspace_path());
    let response = crash_client
        .call("GET", &path, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(response["state"], "indeterminate");
    assert!(response["result"].is_null());
    let before: (bool, i64) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL,session_epoch FROM client_sessions WHERE id=$1",
    )
    .bind(session)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(before, (false, 1));
    f.client
        .call("GET", &path, Some(&f.host), None, None, None)
        .await
        .status(200);
    f.client
        .call("GET", &path, Some(&f.host), None, None, None)
        .await
        .status(200);
    let after: (bool, i64) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL,session_epoch FROM client_sessions WHERE id=$1",
    )
    .bind(session)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(after, (true, 2));
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("crash-before-followup"),
            None,
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some("cannot-rebind-revoked"),
            Some(json!({})),
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.host),
            None,
            Some("replacement-worker-session"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .status(201);
    f.backend.untouched();
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
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.host),
            None,
            Some("recover-worker-session"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .status(201);
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
    let ReceiptEntry::Fresh {
        operation_id: owner_id,
        ..
    } = entry
    else {
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
