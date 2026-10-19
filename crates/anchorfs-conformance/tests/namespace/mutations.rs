//! Interim HTTP coverage; broader namespace suites follow in WP5.
use crate::common::fixture::*;
#[tokio::test]
async fn namespace_mutations_publish_and_replay_receipts() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let directory = format!("{prefix}/directories");
    let body = json!({"path":"a","mode":493});
    let first = f
        .client
        .call(
            "POST",
            &directory,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("mkdir"),
            Some(body.clone()),
        )
        .await
        .status(200)
        .body;
    let replay = f
        .client
        .call(
            "POST",
            &directory,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("mkdir"),
            Some(body),
        )
        .await
        .status(200)
        .body;
    assert_eq!(first, replay);
    f.client
        .call(
            "POST",
            &directory,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("mkdir"),
            Some(json!({"path":"a","mode":448})),
        )
        .await
        .error(409, "idempotency_mismatch");
    let receipt = f
        .client
        .call(
            "GET",
            &format!(
                "{prefix}/operations/{}",
                first["operation_id"].as_str().unwrap()
            ),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(receipt["state"], "committed");
    assert_eq!(receipt["result"], first);
    for (method, suffix, key, body) in [
        (
            "POST",
            "/symlinks",
            "symlink",
            Some(json!({"path":"a/sym","target":"/opaque"})),
        ),
        (
            "POST",
            "/links",
            "link",
            Some(json!({"existing_path":"a/sym","new_path":"a/alias"})),
        ),
        (
            "POST",
            "/rename",
            "rename",
            Some(json!({"from":"a/alias","to":"a/moved"})),
        ),
        (
            "PATCH",
            "/attributes",
            "chmod",
            Some(json!({"path":"a","mode":448})),
        ),
        ("DELETE", "/files?path=a/sym", "unlink1", None),
        ("DELETE", "/files?path=a/moved", "unlink2", None),
        ("DELETE", "/directories?path=a", "rmdir", None),
    ] {
        f.client
            .call(
                method,
                &format!("{prefix}{suffix}"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(key),
                body,
            )
            .await
            .status(200);
        assert!(
            anchorfs_persistence::invariants::check(
                &f.pool,
                anchorfs_core::WorkspaceId(f.workspace)
            )
            .await
            .unwrap()
            .is_empty()
        );
    }
    f.backend.untouched();
}
#[tokio::test]
async fn ns7_cycles_and_ns10_symlink_type_rules() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    for path in ["a", "a/b"] {
        f.client
            .call(
                "POST",
                &format!("{prefix}/directories"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(path),
                Some(json!({"path":path,"mode":493})),
            )
            .await
            .status(200);
    }
    for to in ["a/x", "a/b/c"] {
        f.client
            .call(
                "POST",
                &format!("{prefix}/rename"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(to),
                Some(json!({"from":"a","to":to})),
            )
            .await
            .error(400, "invalid_input");
    }
    f.client
        .call(
            "POST",
            &format!("{prefix}/symlinks"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("sym"),
            Some(json!({"path":"sym","target":"/opaque"})),
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &format!("{prefix}/directories?path=sym"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("rmdir-sym"),
            None,
        )
        .await
        .error(409, "not_directory");
    f.client
        .call(
            "PATCH",
            &format!("{prefix}/attributes"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("chmod-sym"),
            Some(json!({"path":"sym","mode":493})),
        )
        .await
        .error(400, "invalid_input");
    f.client.call("POST",&format!("{prefix}/handles"),Some(&f.worker),Some(&f.worker_session),Some("open-sym"),Some(json!({"path":"sym","read":true,"write":false,"create":false,"exclusive":false,"truncate":false}))).await.error(400,"invalid_input");
    f.invariants().await;
}
#[tokio::test]
async fn ns1_directory_roundtrip_updates_workspace_root() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let before = f
        .client
        .call("GET", &prefix, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    let created = f
        .client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("mkdir"),
            Some(json!({"path":"d","mode":493})),
        )
        .await
        .status(200)
        .body;
    let _: anchorfs_service::dto::MutationResult = serde_json::from_value(created.clone()).unwrap();
    let stat = f
        .client
        .call(
            "GET",
            &format!("{prefix}/stat?path=d"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    let _: anchorfs_service::dto::InodeAttributes = serde_json::from_value(stat.clone()).unwrap();
    assert_eq!(stat["inode_id"], created["inode_id"]);
    let during = f
        .client
        .call("GET", &prefix, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(during["root"]["nlink"], 3);
    assert!(
        during["root"]["generation"].as_i64().unwrap()
            > before["root"]["generation"].as_i64().unwrap()
    );
    f.client
        .call(
            "DELETE",
            &format!("{prefix}/directories?path=d"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("rmdir"),
            None,
        )
        .await
        .status(200);
    let after = f
        .client
        .call("GET", &prefix, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(after["root"]["nlink"], 2);
    assert!(
        after["root"]["generation"].as_i64().unwrap()
            > during["root"]["generation"].as_i64().unwrap()
    );
    f.invariants().await;
}
