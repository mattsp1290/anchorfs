use super::support::*;

#[tokio::test]
async fn by3c_definite_commit_rejection_finalizes_without_publishing() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    sqlx::raw_sql("CREATE FUNCTION test_byte_commit_reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE check_violation USING MESSAGE='synthetic deferred constraint'; END $$; CREATE CONSTRAINT TRIGGER test_byte_commit_reject AFTER INSERT ON manifests DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_byte_commit_reject();").execute(&f.pool).await.unwrap();
    error(
        append(&f, &handle, "rejected", b"data").await,
        503,
        "dependency_unavailable",
    );
    let state: String = sqlx::query_scalar(
        "SELECT state FROM operations WHERE workspace_id=$1 AND idempotency_key='rejected'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(state, "failed");
    assert_eq!(committed(&f, &handle).await, b"");
    let charges:(i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(charges, (4, 0));
    f.invariants().await;
}

#[tokio::test]
async fn by3c_connection_loss_during_manifest_commit_preserves_atomicity() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    sqlx::raw_sql("CREATE FUNCTION test_byte_commit_barrier() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(918272); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER test_byte_commit_barrier AFTER INSERT ON manifests DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_byte_commit_barrier();").execute(&f.pool).await.unwrap();
    let mut holder = f.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(918272)")
        .execute(&mut *holder)
        .await
        .unwrap();
    let request = append_task(&f, &handle, "commit-loss", b"once");
    let pid:i32=tokio::time::timeout(Duration::from_secs(10),async {loop {
        let pid:Option<i32>=sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND query='COMMIT' AND wait_event_type='Lock' AND wait_event='advisory'").fetch_optional(&f.pool).await.unwrap();
        if let Some(pid)=pid {break pid;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(terminated);
    error(request.await.unwrap(), 503, "outcome_unknown");
    sqlx::query("SELECT pg_advisory_unlock(918272)")
        .execute(&mut *holder)
        .await
        .unwrap();
    drop(holder);
    let state: String = sqlx::query_scalar(
        "SELECT state FROM operations WHERE workspace_id=$1 AND idempotency_key='commit-loss'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM manifests WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, if state == "committed" { 2 } else { 1 });
    if state == "pending" {
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM upload_reservations WHERE workspace_id=$1 AND state='active'",
        )
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(active, 1);
        sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
        error(
            append(&f, &handle, "commit-loss", b"once").await,
            409,
            "operation_failed",
        );
    } else {
        assert_eq!(state, "committed");
        assert_eq!(committed(&f, &handle).await, b"once");
    }
    f.invariants().await;
}

#[tokio::test]
async fn by5c_new_router_recovers_a_crashed_pending_upload() {
    let mut f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "seed", 0, b"safe").await.status(200);
    let mut state = AppState::production((*f.config).clone(), f.pool.clone(), f.backend.service());
    state.faults = Arc::new(CrashOnce::at(FaultPoint::BeforePublish));
    f.client = Client {
        app: build_router(state),
    };
    let lost = error(
        write(&f, &handle, "crash", 0, b"lost").await,
        503,
        "outcome_unknown",
    );
    f.client = Client {
        app: build_router(AppState::production(
            (*f.config).clone(),
            f.pool.clone(),
            f.backend.service(),
        )),
    };
    let pending = f
        .client
        .call(
            "GET",
            &format!(
                "{}/operations/{}",
                f.workspace_path(),
                lost["operation_id"].as_str().unwrap()
            ),
            Some(&f.host),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(pending["state"], "pending");
    assert_eq!(
        read(&f, &handle, 0, 4096, None).await.status(200).bytes,
        b"safe".as_slice()
    );
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND state='active'").bind(f.workspace).execute(&f.pool).await.unwrap();
    error(
        write(&f, &handle, "crash", 0, b"lost").await,
        409,
        "operation_failed",
    );
    assert_eq!(
        read(&f, &handle, 0, 4096, None).await.status(200).bytes,
        b"safe".as_slice()
    );
    f.invariants().await;
}

#[tokio::test]
async fn by4_pending_same_key_returns_the_owned_operation_then_replays() {
    let (faults, release) = GateFault::new(FaultPoint::BeforeUpload, 1);
    let f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let first = append_task(&f, &handle, "pending", b"once");
    faults.wait().await;
    let pending = error(
        append(&f, &handle, "pending", b"once").await,
        503,
        "outcome_unknown",
    );
    assert_eq!(committed(&f, &handle).await, b"");
    f.invariants().await;
    release.send(true).unwrap();
    let committed = first.await.unwrap().status(200).json();
    assert_eq!(pending["operation_id"], committed["operation_id"]);
    assert_eq!(
        append(&f, &handle, "pending", b"once")
            .await
            .status(200)
            .json(),
        committed
    );
    assert_eq!(super::support::committed(&f, &handle).await, b"once");
    f.invariants().await;
}

#[tokio::test]
async fn by5_crash_is_durably_fenced_by_same_key_or_quota_pressure() {
    for same_key in [true, false] {
        let mut body = workspace_body();
        body["staging_bytes_limit"] = json!(4);
        let f = Fixture::with_bytes(
            body,
            configure,
            Arc::new(CrashOnce::at(FaultPoint::BeforePublish)),
            false,
        )
        .await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let lost = error(
            append(&f, &handle, "crashed", b"lost").await,
            503,
            "outcome_unknown",
        );
        assert_eq!(committed(&f, &handle).await, b"");
        let state: (String, bool) = sqlx::query_as(
            "SELECT o.state,o.uploaded_at IS NULL FROM objects o WHERE workspace_id=$1",
        )
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(state, ("staged".into(), true));
        let object: Uuid = sqlx::query_scalar("SELECT id FROM objects WHERE workspace_id=$1")
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        let Backend::Minio {
            endpoint,
            deployment,
            ..
        } = &f.backend
        else {
            panic!("MinIO expected")
        };
        let raw = crate::common::minio::raw_client(endpoint).await;
        assert_eq!(
            crate::common::minio::raw_get(&raw, &format!("{deployment}/{}/{object}", f.workspace))
                .await
                .unwrap(),
            b"lost"
        );
        f.invariants().await;
        sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
        if same_key {
            error(
                append(&f, &handle, "crashed", b"lost").await,
                409,
                "operation_failed",
            );
        } else {
            append(&f, &handle, "recovery", b"next").await.status(200);
        }
        let view = f
            .client
            .call(
                "GET",
                &format!(
                    "{}/operations/{}",
                    f.workspace_path(),
                    lost["operation_id"].as_str().unwrap()
                ),
                Some(&f.host),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body;
        assert_eq!(view["state"], "failed");
        assert_eq!(view["result"]["code"], "reservation_expired");
        let values:(String,i64,i64,i64)=sqlx::query_as("SELECT r.state,q.physical_bytes_used,q.staging_bytes_reserved,(SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1) FROM upload_reservations r JOIN workspace_quotas q USING(workspace_id) WHERE r.workspace_id=$1 AND r.operation_id=$2").bind(f.workspace).bind(Uuid::parse_str(lost["operation_id"].as_str().unwrap()).unwrap()).fetch_one(&f.pool).await.unwrap();
        assert_eq!(
            values,
            ("expired".into(), if same_key { 4 } else { 8 }, 0, 1)
        );
        assert_eq!(
            committed(&f, &handle).await,
            if same_key {
                b"".as_slice()
            } else {
                b"next".as_slice()
            }
        );
        f.invariants().await;
    }
}

#[tokio::test]
async fn by7c_reservations_charge_exact_materialized_lengths() {
    let mut body = workspace_body();
    body["staging_bytes_limit"] = json!(1000);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "900", 0, vec![1; 900]).await.status(200);
    error(
        write(&f, &handle, "1100", 4096, vec![2; 1100]).await,
        507,
        "quota_exceeded",
    );
    write(&f, &handle, "950", 900, vec![3; 50])
        .await
        .status(200);
    write(&f, &handle, "1000", 950, vec![4; 50])
        .await
        .status(200);
    error(
        write(&f, &handle, "1050", 950, vec![5; 100]).await,
        507,
        "quota_exceeded",
    );
    let rows:Vec<(String,i64)>=sqlx::query_as("SELECT p.idempotency_key,r.reserved_bytes FROM operations p JOIN upload_reservations r ON r.operation_id=p.id WHERE p.workspace_id=$1 ORDER BY r.reserved_bytes").bind(f.workspace).fetch_all(&f.pool).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("900".into(), 900),
            ("950".into(), 950),
            ("1000".into(), 1000)
        ]
    );
    let mut expected = vec![1; 900];
    expected.extend(vec![3; 50]);
    expected.extend(vec![4; 50]);
    assert_eq!(
        read(&f, &handle, 0, 4096, None).await.status(200).bytes,
        expected
    );
    f.invariants().await;
}

#[tokio::test]
async fn by1b_appends_follow_current_size_including_a_far_write() {
    let mut body = workspace_body();
    body["physical_bytes_limit"] = json!(100_000);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let mut expected = vec![];
    for index in 0..3 {
        let bytes = vec![index as u8 + 1; 1500];
        let result = append(&f, &handle, &format!("append-{index}"), bytes.clone())
            .await
            .status(200)
            .json();
        assert_eq!(result["offset"], expected.len());
        expected.extend(bytes);
        f.invariants().await;
    }
    write(&f, &handle, "far", 6000, b"x").await.status(200);
    expected.resize(6000, 0);
    expected.push(b'x');
    let result = append(&f, &handle, "after-far", b"end")
        .await
        .status(200)
        .json();
    assert_eq!(result["offset"], 6001);
    expected.extend(b"end");
    assert_eq!(committed(&f, &handle).await, expected);
    for offset in [0, 4096] {
        assert_eq!(
            read(&f, &handle, offset, 4096, None)
                .await
                .status(200)
                .bytes,
            expected[offset as usize..expected.len().min(offset as usize + 4096)]
        );
    }
    f.invariants().await;
}
