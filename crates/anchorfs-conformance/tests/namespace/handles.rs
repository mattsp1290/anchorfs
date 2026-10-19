use crate::common::fixture::*;
fn request(path: &str) -> Value {
    json!({"path":path,"read":true,"write":true,"create":true,"exclusive":false,"truncate":false,"mode":384})
}
#[tokio::test]
async fn metadata_handles_preserve_unlinked_and_replaced_inodes() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let a = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("open-a"),
            Some(request("a")),
        )
        .await
        .status(201)
        .body;
    let b = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("open-b"),
            Some(request("b")),
        )
        .await
        .status(201)
        .body;
    let _: anchorfs_service::dto::HandleGrant = serde_json::from_value(a.clone()).unwrap();
    f.client
        .call(
            "POST",
            &format!("{prefix}/rename"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("replace"),
            Some(json!({"from":"a","to":"b"})),
        )
        .await
        .status(200);
    let b_path = format!("{prefix}/handles/{}", b["handle_id"].as_str().unwrap());
    let renewed = f
        .client
        .call(
            "POST",
            &format!("{b_path}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew-b"),
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(
        renewed["attributes"]["inode_id"],
        b["attributes"]["inode_id"]
    );
    assert_eq!(renewed["attributes"]["nlink"], 0);
    f.client
        .call(
            "DELETE",
            &b_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close-b"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &format!("{prefix}/files?path=b"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("unlink-b"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat?path=b"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(404, "not_found");
    let a_path = format!("{prefix}/handles/{}", a["handle_id"].as_str().unwrap());
    f.client
        .call(
            "POST",
            &format!("{a_path}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew-a"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &format!("{a_path}/renew"),
            Some(&f.host),
            Some(&f.host_session),
            Some("foreign"),
            None,
        )
        .await
        .error(404, "not_found");
    f.client
        .call(
            "DELETE",
            &a_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close-a"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &a_path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close-again"),
            None,
        )
        .await
        .error(410, "stale_handle");
    let replay = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("open-a"),
            Some(request("a")),
        )
        .await
        .status(201)
        .body;
    assert_eq!(replay, a);
    assert!(
        anchorfs_persistence::invariants::check(&f.pool, anchorfs_core::WorkspaceId(f.workspace))
            .await
            .unwrap()
            .is_empty()
    );
    let used: i64 = sqlx::query_scalar(
        "SELECT logical_inodes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(used, 0);
    f.backend.untouched();
}
#[tokio::test]
async fn truncate_open_changes_only_inode_generation() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let a = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("create"),
            Some(request("f")),
        )
        .await
        .status(201)
        .body;
    let generation: i64 = sqlx::query_scalar(
        "SELECT namespace_generation FROM namespace_locks WHERE workspace_id=$1",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let mut truncate = request("f");
    truncate["create"] = json!(false);
    truncate["truncate"] = json!(true);
    truncate.as_object_mut().unwrap().remove("mode");
    let b = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("truncate"),
            Some(truncate),
        )
        .await
        .status(201)
        .body;
    assert_eq!(
        b["attributes"]["generation"].as_i64().unwrap(),
        a["attributes"]["generation"].as_i64().unwrap() + 1
    );
    let after: i64 = sqlx::query_scalar(
        "SELECT namespace_generation FROM namespace_locks WHERE workspace_id=$1",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(after, generation);
    assert!(
        anchorfs_persistence::invariants::check(&f.pool, anchorfs_core::WorkspaceId(f.workspace))
            .await
            .unwrap()
            .is_empty()
    );
    f.backend.untouched();
}
#[tokio::test]
async fn ns11_handle_limits_stale_fences_and_concurrent_closes() {
    let f = Fixture::with(workspace_body(), |c| {
        c.max_handles_per_session = 2;
        c.handle_ttl = Duration::from_secs(1);
    })
    .await;
    let prefix = f.workspace_path();
    let mut grants = vec![];
    for (name, status) in [("a", 201), ("b", 201), ("c", 507)] {
        let r = f
            .client
            .call(
                "POST",
                &format!("{prefix}/handles"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(name),
                Some(request(name)),
            )
            .await;
        if status == 201 {
            grants.push(r.status(201).body);
        } else {
            r.error(507, "quota_exceeded");
        }
    }
    let handle = format!(
        "{prefix}/handles/{}",
        grants[0]["handle_id"].as_str().unwrap()
    );
    let (a, b) = tokio::join!(
        f.client.call(
            "DELETE",
            &handle,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close1"),
            None
        ),
        f.client.call(
            "DELETE",
            &handle,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close2"),
            None
        )
    );
    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 410]);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let b = format!(
        "{prefix}/handles/{}",
        grants[1]["handle_id"].as_str().unwrap()
    );
    f.client
        .call(
            "POST",
            &format!("{b}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("expired"),
            None,
        )
        .await
        .error(410, "stale_handle");
    sqlx::query("UPDATE client_sessions SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    f.client
        .call(
            "POST",
            &format!("{b}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("revoked"),
            None,
        )
        .await
        .error(409, "fenced");
    f.invariants().await;
}
#[tokio::test]
async fn ns6_replaced_open_inode_keeps_quota_until_close() {
    let mut body = workspace_body();
    body["logical_inodes_limit"] = json!(2);
    let f = Fixture::with(body, |_| {}).await;
    let prefix = f.workspace_path();
    let a = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("a"),
            Some(request("a")),
        )
        .await
        .status(201)
        .body;
    let b = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("b"),
            Some(request("b")),
        )
        .await
        .status(201)
        .body;
    f.client
        .call(
            "POST",
            &format!("{prefix}/rename"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("replace"),
            Some(json!({"from":"a","to":"b"})),
        )
        .await
        .status(200);
    let stat = f
        .client
        .call(
            "GET",
            &format!("{prefix}/stat?path=b"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(stat["inode_id"], a["attributes"]["inode_id"]);
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("before-close"),
            Some(json!({"path":"new","mode":493})),
        )
        .await
        .error(507, "quota_exceeded");
    f.invariants().await;
    let handle = format!("{prefix}/handles/{}", b["handle_id"].as_str().unwrap());
    f.client
        .call(
            "POST",
            &format!("{handle}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &handle,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("after-close"),
            Some(json!({"path":"new","mode":493})),
        )
        .await
        .status(200);
    f.invariants().await;
}
#[tokio::test]
async fn invalid_open_flags_keep_request_trace_and_publish_nothing() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    for (key, body) in [
        (
            "none",
            json!({"path":"a","read":false,"write":false,"create":false,"exclusive":false,"truncate":false}),
        ),
        (
            "mode",
            json!({"path":"a","read":true,"write":false,"create":true,"exclusive":false,"truncate":false}),
        ),
        (
            "truncate",
            json!({"path":"a","read":true,"write":false,"create":false,"exclusive":false,"truncate":true}),
        ),
    ] {
        f.client
            .call(
                "POST",
                &format!("{prefix}/handles"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(key),
                Some(body),
            )
            .await
            .error(400, "invalid_input");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.invariants().await;
}
