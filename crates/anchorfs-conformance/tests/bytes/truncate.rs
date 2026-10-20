use super::support::*;
use crate::common::proxy;

#[tokio::test]
async fn by3_truncate_response_abandonment_resolves_by_receipt_and_replay() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "seed", 0, b"data").await.status(200);
    let committed = Arc::new(tokio::sync::Notify::new());
    let observed = committed.clone();
    let client = Client {
        app: f.client.app.clone().layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let observed = observed.clone();
                async move {
                    let response = next.run(req).await;
                    // Hold only the HTTP response, after the real truncate transaction.
                    observed.notify_one();
                    std::future::pending::<()>().await;
                    response
                }
            },
        )),
    };
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!("{}/truncate", handle_path(&f, &handle));
    let request = tokio::spawn(async move {
        client
            .call(
                "POST",
                &path,
                Some(&actor),
                Some(&session),
                Some("lost-truncate"),
                Some(json!({"size":2})),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), committed.notified())
        .await
        .unwrap();
    request.abort();
    assert!(matches!(request.await,Err(error) if error.is_cancelled()));
    let receipt = f
        .client
        .call(
            "GET",
            &format!(
                "{}/operations?idempotency_key=lost-truncate",
                f.workspace_path()
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
    assert_eq!(
        resize(&f, &handle, 2, "lost-truncate").await,
        receipt["result"]
    );
    assert_eq!(
        read(&f, &handle, 0, 4096, None).await.status(200).bytes,
        b"da".as_slice()
    );
    f.invariants().await;
}

async fn resize(f: &Fixture, handle: &Value, size: u64, key: &str) -> Value {
    f.client
        .call(
            "POST",
            &format!("{}/truncate", handle_path(f, handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            Some(key),
            Some(json!({"size":size})),
        )
        .await
        .status(200)
        .body
}
async fn matches(f: &Fixture, handle: &Value, expected: &[u8]) {
    for offset in (0..=expected.len()).step_by(4096) {
        assert_eq!(
            read(f, handle, offset as u64, 4096, None)
                .await
                .status(200)
                .bytes,
            expected[offset..expected.len().min(offset + 4096)]
        );
    }
    assert_eq!(committed(f, handle).await, expected);
    f.invariants().await;
}
#[tokio::test]
async fn by10_truncate_never_puts_and_unlinked_handles_still_publish() {
    let mut body = workspace_body();
    body["physical_bytes_limit"] = json!(100_000);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let mut expected: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
    write(&f, &handle, "a", 0, &expected[..4096])
        .await
        .status(200);
    write(&f, &handle, "b", 4096, &expected[4096..])
        .await
        .status(200);
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected")
    };
    let puts = proxy.count(proxy::is_put);
    let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM objects WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut last_generation = 3;
    for (index, size) in [7000, 4000, 0, 9000, 9000].into_iter().enumerate() {
        let old_size = expected.len();
        let value = resize(&f, &handle, size, &format!("truncate-{index}")).await;
        if old_size != size as usize {
            last_generation += 1;
        }
        assert_eq!(value["generation"], last_generation);
        expected.resize(size as usize, 0);
        matches(&f, &handle, &expected).await;
        assert_eq!(proxy.count(proxy::is_put), puts);
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM objects WHERE workspace_id=$1")
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(after, objects);
    }
    f.client
        .call(
            "POST",
            &format!("{}/truncate", handle_path(&f, &handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("too-large"),
            Some(json!({"size":anchorfs_core::Limits::default().max_file_size+1})),
        )
        .await
        .error(413, "file_too_large");
    f.client
        .call(
            "DELETE",
            &format!("{}/files?path=f", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("unlink"),
            None,
        )
        .await
        .status(200);
    resize(&f, &handle, 2, "unlinked-truncate").await;
    write(&f, &handle, "unlinked-write", 1, b"x")
        .await
        .status(200);
    matches(&f, &handle, b"\0x").await;
}
#[tokio::test]
async fn by10b_truncate_on_open_preserves_old_pin_and_releases_logical_bytes() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let old = write(&f, &handle, "data", 0, b"old bytes")
        .await
        .status(200)
        .json();
    let pin = pin(&f, "f", "pin").await;
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected")
    };
    let puts = proxy.count(proxy::is_put);
    let opened=f.client.call("POST",&format!("{}/handles",f.workspace_path()),Some(&f.worker),Some(&f.worker_session),Some("truncate-open"),Some(json!({"path":"f","read":true,"write":true,"create":false,"exclusive":false,"truncate":true,"mode":420}))).await.status(201).body;
    assert_eq!(
        opened["attributes"]["generation"].as_i64().unwrap(),
        old["generation"].as_i64().unwrap() + 1
    );
    assert!(
        read(&f, &opened, 0, 4096, None)
            .await
            .status(200)
            .bytes
            .is_empty()
    );
    let old_read = read(
        &f,
        &handle,
        0,
        4096,
        Some(pin["read_pin_id"].as_str().unwrap().parse().unwrap()),
    )
    .await
    .status(200);
    assert_eq!(old_read.bytes, b"old bytes".as_slice());
    assert_eq!(
        old_read.headers["X-Anchorfs-Generation"].to_str().unwrap(),
        old["generation"].as_i64().unwrap().to_string()
    );
    let logical: i64 =
        sqlx::query_scalar("SELECT logical_bytes_used FROM workspace_quotas WHERE workspace_id=$1")
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(logical, 0);
    assert_eq!(proxy.count(proxy::is_put), puts);
    f.invariants().await;
}
#[tokio::test]
async fn by11_shrink_then_far_write_never_resurrects_trimmed_tails() {
    let mut body = workspace_body();
    body["physical_bytes_limit"] = json!(100_000);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "seed", 0, vec![1; 4096])
        .await
        .status(200);
    resize(&f, &handle, 4000, "shrink").await;
    write(&f, &handle, "far", 8000, vec![2; 100])
        .await
        .status(200);
    let mut expected = vec![1; 4000];
    expected.resize(8000, 0);
    expected.extend(vec![2; 100]);
    matches(&f, &handle, &expected).await;
    resize(&f, &handle, 3990, "shrink-again").await;
    resize(&f, &handle, 8100, "regrow").await;
    expected.truncate(3990);
    expected.resize(8100, 0);
    matches(&f, &handle, &expected).await;
    write(&f, &handle, "trimmed-write", 4000, b"XYZ")
        .await
        .status(200);
    expected[4000..4003].copy_from_slice(b"XYZ");
    matches(&f, &handle, &expected).await;
}
