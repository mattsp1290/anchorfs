use super::support::*;
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
