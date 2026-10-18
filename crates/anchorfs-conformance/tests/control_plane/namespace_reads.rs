use super::support::*;
#[tokio::test]
async fn namespace_reads_page_in_byte_order_and_bind_cursors() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    for (i, name) in ["B", "a", "é", "_", "Z"].iter().enumerate() {
        f.client
            .call(
                "POST",
                &format!("{prefix}/directories"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(&format!("mkdir{i}")),
                Some(json!({"path":name,"mode":493})),
            )
            .await
            .status(200);
    }
    let stat = f
        .client
        .call(
            "GET",
            &format!("{prefix}/stat?path=B"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(stat["kind"], "directory");
    assert_eq!(stat["nlink"], 2);
    let mut names = vec![];
    let mut cursor = None;
    loop {
        let url = format!(
            "{prefix}/directory?limit=2{}",
            cursor
                .as_ref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let page = f
            .client
            .call(
                "GET",
                &url,
                Some(&f.worker),
                Some(&f.worker_session),
                None,
                None,
            )
            .await
            .status(200)
            .body;
        let _: anchorfs_service::dto::DirectoryPage = serde_json::from_value(page.clone()).unwrap();
        names.extend(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["name"].as_str().unwrap().to_owned()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if let Some(cursor) = &cursor {
            let error = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/directory?path=B&cursor={cursor}"),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    None,
                    None,
                )
                .await
                .error(400, "invalid_input");
            assert_eq!(error.body["message"], "cursor_scope");
            let mut bad = cursor.clone();
            bad.pop();
            bad.push('g');
            let error = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/directory?cursor={bad}"),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    None,
                    None,
                )
                .await
                .error(400, "invalid_input");
            assert_eq!(error.body["message"], "cursor_invalid");
        } else {
            break;
        }
    }
    assert_eq!(names, ["B", "Z", "_", "a", "é"]);
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
    let read = f
        .client
        .call(
            "GET",
            &format!("{prefix}/symlinks?path=sym"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(read["target"], "/opaque");
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat?path=sym/child"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(409, "not_directory");
    assert!(
        anchorfs_persistence::invariants::check(&f.pool, anchorfs_core::WorkspaceId(f.workspace))
            .await
            .unwrap()
            .is_empty()
    );
    f.backend.untouched();
}
