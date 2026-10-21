use super::support::*;

#[tokio::test]
async fn concurrent_acquires_have_one_winner() {
    let mut f = Fixture::new().await;
    f.client = f.concurrent_control_client(8).await;
    let before: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let b = f.credential("host", "write", 1000, 1000).await;
    let mut actors = Vec::new();
    for actor in [&f.host, &b] {
        for _ in 0..4 {
            actors.push((actor.clone(), f.session_for(actor).await));
        }
    }
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM workspaces WHERE id=$1 FOR SHARE")
        .bind(f.workspace)
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for (actor, session) in &actors {
        let c = Client {
            app: f.client.app.clone(),
        };
        let path = lease_path(&f, "");
        let actor = actor.clone();
        let session = session.clone();
        tasks.push(tokio::spawn(async move {
            c.call(
                "POST",
                &path,
                Some(&actor),
                Some(&session),
                Some(&Uuid::new_v4().to_string()),
                None,
            )
            .await
        }));
    }
    // Accept either the intended exclusive SELECT or an erroneous later UPDATE:
    // a SHARE-then-upgrade regression must reach the contested lock too.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='active' AND wait_event_type='Lock' AND query ILIKE '%workspaces%' AND (query ILIKE '%FOR UPDATE%' OR query ILIKE '%UPDATE workspaces%')")
                .fetch_one(&f.pool).await.unwrap();
            if waiting==8 {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("all eight HTTP requests reached workspace contention");
    holder.commit().await.unwrap();
    let mut winner = None;
    let mut losses = 0;
    for (i, task) in tasks.into_iter().enumerate() {
        let r = task.await.unwrap();
        if r.status == 201 {
            assert!(winner.is_none());
            winner = Some((i, r.body));
        } else {
            message(r, 409, "workspace_busy", "lease_active");
            losses += 1;
        }
    }
    assert_eq!(losses, 7);
    let (i, g) = winner.unwrap();
    let owner = &actors[i].0;
    let mut tasks = Vec::new();
    for (actor, session) in &actors {
        if actor.id != owner.id {
            continue;
        }
        let c = granted(&f, &g);
        let path = lease_path(&f, "");
        let actor = actor.clone();
        let session = session.clone();
        tasks.push(tokio::spawn(async move {
            c.call(
                "DELETE",
                &path,
                Some(&actor),
                Some(&session),
                Some(&Uuid::new_v4().to_string()),
                None,
            )
            .await
        }));
    }
    let mut wins = 0;
    for task in tasks {
        let r = task.await.unwrap();
        if r.status == 200 {
            wins += 1;
        } else {
            message(r, 409, "workspace_busy", "lease_absent");
        }
    }
    assert_eq!(wins, 1);
    let g = lease(&f, &f.client, &f.host, &f.host_session, "", "again")
        .await
        .status(201)
        .body;
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let c = Client {
            app: f.client.app.clone(),
        };
        let path = format!("{}/sessions", f.workspace_path());
        let actor = f.host.clone();
        let body = json!({"credential_id":f.worker.id,"bind_to_workflow_lease":g["token"]});
        tasks.push(tokio::spawn(async move {
            c.call(
                "POST",
                &path,
                Some(&actor),
                None,
                Some(&Uuid::new_v4().to_string()),
                Some(body),
            )
            .await
        }));
    }
    lease(
        &f,
        &granted(&f, &g),
        &f.host,
        &f.host_session,
        "release",
        "race-release",
    )
    .await
    .status(200);
    let epoch = workspace_info(&f, &f.host).await["mutation_epoch"]
        .as_i64()
        .unwrap();
    for task in tasks {
        let r = task.await.unwrap();
        if r.status == 201 {
            assert!(r.body["bound_mutation_epoch"].as_i64().unwrap() <= epoch);
        } else {
            message(r, 409, "workspace_busy", "lease_absent");
        }
    }
    sqlx::query("SELECT pg_stat_force_next_flush()")
        .execute(&f.pool)
        .await
        .unwrap();
    let mut stats =
        <sqlx::PgConnection as sqlx::Connection>::connect_with(&f.pool.connect_options())
            .await
            .unwrap();
    let after: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&mut stats)
    .await
    .unwrap();
    assert_eq!(before, after);
    f.invariants().await;
}
#[tokio::test]
async fn lease_pool_wait_past_deadline_does_not_create_an_abandoned_grant() {
    let f = Fixture::new().await;
    for late in [true, false] {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with((*f.pool.connect_options()).clone())
            .await
            .unwrap();
        let (mut state, _backend) = crate::common::state_fake(pool.clone());
        let mut config = (*state.config).clone();
        config.metadata_call_deadline = Duration::from_millis(600);
        state.config = Arc::new(config);
        state.control_permits = Arc::new(tokio::sync::Semaphore::new(1));
        let permits = state.control_permits.clone();
        let client = Client {
            app: build_router(state),
        };
        let held = pool.acquire().await.unwrap();
        let actor = f.host.clone();
        let session = f.host_session.clone();
        let path = lease_path(&f, "");
        let key = if late {
            "late-pool-grant"
        } else {
            "early-pool-grant"
        };
        let task = tokio::spawn(async move {
            client
                .call("POST", &path, Some(&actor), Some(&session), Some(key), None)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while permits.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if late {
            task.await.unwrap().error(503, "outcome_unknown");
            // Release after the HTTP response but before the pool's own timeout.
            drop(held);
            let _finished = tokio::time::timeout(Duration::from_secs(2), permits.acquire())
                .await
                .unwrap()
                .unwrap();
            let (epoch, recorded): (i64, bool) = sqlx::query_as(
                "SELECT mutation_epoch,workflow_expires_at IS NOT NULL FROM workspaces WHERE id=$1",
            )
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!((epoch, recorded), (1, false));
            let receipts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key=$2",
            )
            .bind(f.workspace)
            .bind(key)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(receipts, 0);
        } else {
            drop(held);
            let grant = task.await.unwrap().status(201).body;
            assert!(grant["token"].is_string());
            assert_eq!(grant["mutation_epoch"], 2);
        }
    }
    f.invariants().await;
}
