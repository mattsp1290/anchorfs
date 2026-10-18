use super::support::*;

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
