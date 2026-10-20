use super::support::*;

#[tokio::test]
async fn by16_handle_access_and_administrator_replace_are_enforced() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let original = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let mut handles = vec![];
    for (read, write) in [(true, false), (false, true)] {
        let handle=f.client.call("POST",&format!("{}/handles",f.workspace_path()),Some(&f.worker),Some(&f.worker_session),Some(&Uuid::new_v4().to_string()),Some(json!({"path":"f","read":read,"write":write,"create":false,"exclusive":false,"truncate":false,"mode":420}))).await.status(201).body;
        handles.push(handle);
    }
    error(
        write(&f, &handles[0], "read-only", 0, b"no").await,
        403,
        "forbidden",
    );
    error(read(&f, &handles[1], 0, 4096, None).await, 403, "forbidden");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key='read-only'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    let attrs = f
        .client
        .call(
            "PATCH",
            &format!("{}/attributes", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("zero-mode"),
            Some(json!({"path":"f","mode":0})),
        )
        .await
        .status(200)
        .body;
    let bytes = b"admin";
    let digest = anchorfs_core::sha256(bytes).to_string();
    let administrator = f.credential("host", "admin", 2000, 2000).await;
    let administrator_session = f.session_for(&administrator).await;
    let replaced = f
        .client
        .binary(
            "PUT",
            &format!(
                "{}/files?path=f&expected_generation={}",
                f.workspace_path(),
                attrs["generation"]
            ),
            Some(&administrator),
            Some(&administrator_session),
            Some("admin-replace"),
            &[
                ("Content-Length", "5"),
                ("X-Anchorfs-Content-SHA256", &digest),
            ],
            Body::from(bytes.as_slice()),
        )
        .await
        .status(200)
        .json();
    assert_eq!(
        replaced["generation"].as_i64().unwrap(),
        attrs["generation"].as_i64().unwrap() + 1
    );
    assert_eq!(committed(&f, &original).await, bytes);
    f.invariants().await;
}

#[tokio::test]
async fn by14_replace_rejects_directory_symlink_and_read_role_before_admission() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    open(&f, &f.worker, &f.worker_session, "f", true).await;
    for (suffix, body) in [
        ("directories", json!({"path":"dir","mode":493})),
        ("symlinks", json!({"path":"link","target":"f"})),
    ] {
        f.client
            .call(
                "POST",
                &format!("{}/{suffix}", f.workspace_path()),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(suffix),
                Some(body),
            )
            .await
            .status(200);
    }
    error(
        replace(&f, "dir", 1, "directory", b"x").await,
        409,
        "is_directory",
    );
    error(
        replace(&f, "link", 1, "symlink", b"x").await,
        400,
        "invalid_input",
    );
    let reader = f.credential("worker", "read", 1000, 1000).await;
    let session = f.session_for(&reader).await;
    let digest = anchorfs_core::sha256(b"x").to_string();
    error(
        f.client
            .binary(
                "PUT",
                &format!("{}/files?path=f&expected_generation=1", f.workspace_path()),
                Some(&reader),
                Some(&session),
                Some("read-role"),
                &[
                    ("Content-Length", "1"),
                    ("X-Anchorfs-Content-SHA256", &digest),
                ],
                Body::from("x"),
            )
            .await,
        403,
        "forbidden",
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key IN ('directory','symlink','read-role')").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 0);
    f.invariants().await;
}

#[tokio::test]
async fn by13_real_pin_ttl_and_by15_expired_fsync_use_database_time() {
    let f = Fixture::with_bytes(
        workspace_body(),
        |c| {
            configure(c);
            c.read_pin_ttl = Duration::from_secs(1);
        },
        Arc::new(NoFaults),
        false,
    )
    .await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "data", 0, b"data").await.status(200);
    let pin = pin(&f, "f", "pin").await;
    let pin_id: Uuid = pin["read_pin_id"].as_str().unwrap().parse().unwrap();
    let expiry: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM read_pins WHERE id=$1")
            .bind(pin_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    wait_until_expired(&f.pool, expiry).await;
    error(
        read(&f, &handle, 0, 4, Some(pin_id)).await,
        410,
        "stale_handle",
    );
    assert_eq!(
        read(&f, &handle, 0, 4, None).await.status(200).bytes,
        b"data".as_slice()
    );
    sqlx::query(
        "UPDATE handles SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1",
    )
    .bind(f.workspace)
    .execute(&f.pool)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let reply = f
        .client
        .call(
            "POST",
            &format!("{}/fsync", handle_path(&f, &handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(410)
        .body;
    assert_eq!(reply["code"], "stale_handle");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    f.invariants().await;
}
