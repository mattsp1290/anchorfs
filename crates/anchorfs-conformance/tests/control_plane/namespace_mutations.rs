//! Interim HTTP coverage; broader namespace suites follow in WP5.
use super::support::*;
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
