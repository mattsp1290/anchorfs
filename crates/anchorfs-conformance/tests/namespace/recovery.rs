use crate::common::fixture::*;
#[tokio::test]
async fn ns4_quota_failure_publishes_nothing() {
    let mut body = workspace_body();
    body["logical_inodes_limit"] = json!(2);
    let f = Fixture::with(body, |_| {}).await;
    let prefix = f.workspace_path();
    for (name, status) in [("a", 200), ("b", 200), ("c", 507)] {
        let r = f
            .client
            .call(
                "POST",
                &format!("{prefix}/directories"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(name),
                Some(json!({"path":name,"mode":493})),
            )
            .await;
        if status == 507 {
            r.error(507, "quota_exceeded");
        } else {
            r.status(status);
        }
        f.invariants().await;
    }
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat?path=c"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(404, "not_found");
    let usage: i64 = sqlx::query_scalar(
        "SELECT logical_inodes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(usage, 2);
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key='c'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0);
}
#[tokio::test]
async fn ns3b_expired_receipt_is_not_reapplied() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let body = json!({"path":"a","mode":493});
    let first = f
        .client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("expired"),
            Some(body.clone()),
        )
        .await
        .status(200)
        .body;
    sqlx::query("UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND idempotency_key='expired'").bind(f.workspace).execute(&f.pool).await.unwrap();
    let r = f
        .client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("expired"),
            Some(body),
        )
        .await
        .error(409, "operation_indeterminate");
    assert_eq!(r.body["operation_id"], first["operation_id"]);
    let receipt = f
        .client
        .call(
            "GET",
            &format!("{prefix}/operations?idempotency_key=expired"),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(receipt["state"], "indeterminate");
    assert!(receipt["result"].is_null());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM directory_entries WHERE workspace_id=$1 AND name='a'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    f.invariants().await;
}
#[tokio::test]
async fn ns3_lost_response_resolves_and_replays_once() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let client = f.client_with_deadline(400);
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT workspace_id FROM namespace_locks WHERE workspace_id=$1 FOR UPDATE")
        .bind(f.workspace)
        .execute(&mut *holder)
        .await
        .unwrap();
    let worker = f.worker.clone();
    let session = f.worker_session.clone();
    let url = format!("{prefix}/directories");
    let request = tokio::spawn(async move {
        client
            .call(
                "POST",
                &url,
                Some(&worker),
                Some(&session),
                Some("lost"),
                Some(json!({"path":"a","mode":493})),
            )
            .await
    });
    wait_for_lock(&f.pool, "SELECT namespace_generation FROM namespace_locks").await;
    request.await.unwrap().error(503, "outcome_unknown");
    holder.commit().await.unwrap();
    let receipt = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/operations?idempotency_key=lost"),
                    Some(&f.worker),
                    None,
                    None,
                    None,
                )
                .await;
            if r.status == axum::http::StatusCode::OK {
                break r.body;
            }
            r.status(404);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(receipt["state"], "committed");
    let replay = f
        .client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("lost"),
            Some(json!({"path":"a","mode":493})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(replay, receipt["result"]);
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat?path=a"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200);
    f.invariants().await;
}
