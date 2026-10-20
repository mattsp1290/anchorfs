use super::support::*;

#[tokio::test]
async fn by21_pool_saturation_keeps_an_ambiguous_receipt_recoverable() {
    for held in [2, 4] {
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let mut f = Fixture::with_bytes(
            workspace_body(),
            |c| {
                configure(c);
                c.db_max_connections = 4;
                c.max_data_ops = 2;
                c.data_call_deadline = Duration::from_millis(1500);
                c.db_lock_timeout = Duration::from_millis(100);
            },
            faults.clone(),
            false,
        )
        .await;
        let mut state =
            AppState::production((*f.config).clone(), f.pool.clone(), f.backend.service());
        state.faults = faults.clone();
        let permits = state.data_permits.clone();
        f.client = Client {
            app: build_router(state),
        };
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let request = append_task(&f, &handle, "pool", b"data");
        faults.wait().await;
        let mut connections = vec![];
        for _ in 0..held {
            connections.push(f.pool.acquire().await.unwrap());
        }
        release.send(true).unwrap();
        let response = request.await.unwrap();
        if held == 2 {
            response.status(200);
            assert_eq!(committed(&f, &handle).await, b"data");
        } else {
            error(response, 503, "outcome_unknown");
            tokio::time::timeout(Duration::from_secs(3), async {
                while permits.available_permits() != 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("detached finalization ends within bounded grace");
            drop(connections);
            let state: String = sqlx::query_scalar(
                "SELECT state FROM operations WHERE workspace_id=$1 AND idempotency_key='pool'",
            )
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(state, "pending");
            sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
            error(
                append(&f, &handle, "pool", b"data").await,
                409,
                "operation_failed",
            );
            assert_eq!(committed(&f, &handle).await, b"");
            f.invariants().await;
            continue;
        }
        drop(connections);
        f.invariants().await;
    }
}
