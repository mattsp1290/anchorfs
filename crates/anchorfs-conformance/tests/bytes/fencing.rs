use super::support::*;

#[tokio::test]
async fn by9_reservation_expiry_during_quota_wait_fences_every_upload_publication() {
    for mode in ["write", "append", "replace"] {
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), false).await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let client = Client {
            app: f.client.app.clone(),
        };
        let actor = f.worker.clone();
        let session = f.worker_session.clone();
        let path = match mode {
            "write" => format!("{}/bytes?offset=0", handle_path(&f, &handle)),
            "append" => format!("{}/append", handle_path(&f, &handle)),
            _ => format!("{}/files?path=f&expected_generation=1", f.workspace_path()),
        };
        let pending = tokio::spawn(async move {
            let digest = anchorfs_core::sha256(b"late").to_string();
            let headers = if mode == "replace" {
                vec![
                    ("Content-Length", "4"),
                    ("X-Anchorfs-Content-SHA256", digest.as_str()),
                ]
            } else {
                vec![]
            };
            client
                .binary(
                    if mode == "append" { "POST" } else { "PUT" },
                    &path,
                    Some(&actor),
                    Some(&session),
                    Some("late"),
                    &headers,
                    Body::from("late"),
                )
                .await
        });
        faults.wait().await;
        let expiry: time::OffsetDateTime = sqlx::query_scalar(
            "UPDATE upload_reservations SET expires_at=clock_timestamp()+interval '2 seconds' WHERE workspace_id=$1 AND state='active' RETURNING expires_at"
        ).bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        let mut quota_lock = f.pool.begin().await.unwrap();
        sqlx::query("SELECT workspace_id FROM workspace_quotas WHERE workspace_id=$1 FOR UPDATE")
            .bind(f.workspace)
            .execute(&mut *quota_lock)
            .await
            .unwrap();
        release.send(true).unwrap();
        // This statement is reached after lock_active and consume. The clock
        // must still be checked after this later lock wait, before COMMIT.
        wait_for_lock(&f.pool, "UPDATE workspace_quotas").await;
        wait_until_expired(&f.pool, expiry).await;
        quota_lock.rollback().await.unwrap();
        let failed = error(pending.await.unwrap(), 409, "fenced");
        assert_eq!(failed["message"], "reservation_expired");
        let old = read(&f, &handle, 0, 4096, None).await.status(200);
        assert!(old.bytes.is_empty());
        assert_eq!(old.headers["X-Anchorfs-Generation"], "1");
        let receipt = f
            .client
            .call(
                "GET",
                &format!(
                    "{}/operations/{}",
                    f.workspace_path(),
                    failed["operation_id"].as_str().unwrap()
                ),
                Some(&f.host),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body;
        assert_eq!(receipt["state"], "failed");
        assert_eq!(receipt["result"]["code"], "reservation_expired");
        let charges: (String, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT r.state,q.physical_bytes_used,q.staging_bytes_reserved,(SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1),(SELECT count(*) FROM manifests WHERE workspace_id=$1) FROM upload_reservations r JOIN workspace_quotas q USING(workspace_id) WHERE r.workspace_id=$1"
        ).bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(charges, ("failed".into(), 4, 0, 1, 1), "{mode}");
        f.invariants().await;
    }
}

#[tokio::test]
async fn by3b_expired_byte_receipts_fence_sessions_but_failed_receipts_do_not() {
    for failed in [false, true] {
        let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let operation = if failed {
            let digest = anchorfs_core::sha256(b"next").to_string();
            error(
                f.client
                    .binary(
                        "PUT",
                        &format!("{}/files?path=f&expected_generation=1", f.workspace_path()),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some("expired"),
                        &[
                            ("Content-Length", "4"),
                            ("X-Anchorfs-Content-SHA256", &digest),
                        ],
                        Body::from("oops"),
                    )
                    .await,
                400,
                "invalid_input",
            )
        } else {
            write(&f, &handle, "expired", 0, b"data")
                .await
                .status(200)
                .json()
        };
        let operation_id: Uuid = operation["operation_id"].as_str().unwrap().parse().unwrap();
        sqlx::query(
            "UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
        )
        .bind(operation_id)
        .execute(&f.pool)
        .await
        .unwrap();
        if !failed {
            error(
                write(&f, &handle, "expired", 0, b"data").await,
                409,
                "operation_indeterminate",
            );
        }
        let view = f
            .client
            .call(
                "GET",
                &format!("{}/operations/{operation_id}", f.workspace_path()),
                Some(&f.host),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body;
        assert_eq!(view["state"], "indeterminate");
        assert!(view["result"].is_null());
        assert_eq!(
            committed(&f, &handle).await,
            if failed {
                b"".as_slice()
            } else {
                b"data".as_slice()
            }
        );
        let reply = f
            .client
            .call(
                "GET",
                &format!("{}/stat?path=f", f.workspace_path()),
                Some(&f.worker),
                Some(&f.worker_session),
                None,
                None,
            )
            .await;
        if failed {
            reply.status(200);
            write(&f, &handle, "new", 0, b"safe").await.status(200);
        } else {
            reply.error(409, "fenced");
            f.client
                .call(
                    "POST",
                    &format!("{}/renew", handle_path(&f, &handle)),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    Some("handle-renew"),
                    None,
                )
                .await
                .error(409, "fenced");
            f.client
                .call(
                    "POST",
                    &f.session_path(&f.worker_session, "/renew"),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    Some("session-renew"),
                    None,
                )
                .await
                .error(409, "fenced");
            f.client
                .call(
                    "POST",
                    &f.session_path(&f.worker_session, "/rebind"),
                    Some(&f.host),
                    Some(&f.host_session),
                    Some("session-rebind"),
                    Some(json!({})),
                )
                .await
                .error(409, "fenced");
            let fresh = f.session_for(&f.worker).await;
            f.client
                .call(
                    "GET",
                    &format!("{}/stat?path=f", f.workspace_path()),
                    Some(&f.worker),
                    Some(&fresh),
                    None,
                    None,
                )
                .await
                .status(200);
        }
        f.invariants().await;
    }
}

#[tokio::test]
async fn by9_publication_rechecks_rotation_authorization_expiry_and_lease() {
    for mode in [
        "rotation",
        "authorization",
        "handle-ttl",
        "reservation",
        "lease",
    ] {
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let f = Fixture::with_bytes(
            workspace_body(),
            |c| {
                configure(c);
                if mode == "handle-ttl" {
                    c.handle_ttl = Duration::from_secs(1);
                }
            },
            faults.clone(),
            false,
        )
        .await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let pending = append_task(&f, &handle, "pending", b"data");
        faults.wait().await;
        let (status, code) = match mode {
            "rotation" => {
                f.client
                    .call(
                        "POST",
                        &format!("/v1/credentials/{}/rotate", f.worker.id),
                        Some(&f.host),
                        None,
                        Some("rotation"),
                        None,
                    )
                    .await
                    .status(200);
                (409, "fenced")
            }
            "authorization" => {
                sqlx::query(
                    "UPDATE workspaces SET authorization_epoch=authorization_epoch+1 WHERE id=$1",
                )
                .bind(f.workspace)
                .execute(&f.pool)
                .await
                .unwrap();
                (409, "fenced")
            }
            "handle-ttl" => {
                let expiry: time::OffsetDateTime =
                    sqlx::query_scalar("SELECT expires_at FROM handles WHERE workspace_id=$1")
                        .bind(f.workspace)
                        .fetch_one(&f.pool)
                        .await
                        .unwrap();
                wait_until_expired(&f.pool, expiry).await;
                (410, "stale_handle")
            }
            "reservation" => {
                sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
                (409, "fenced")
            }
            "lease" => {
                sqlx::query("UPDATE workspaces SET workflow_owner_credential_id=$2,workflow_token_hash=$3,workflow_expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1").bind(f.workspace).bind(f.host.id).bind(anchorfs_core::sha256([12;32]).0.as_slice()).execute(&f.pool).await.unwrap();
                (409, "workspace_busy")
            }
            _ => unreachable!(),
        };
        release.send(true).unwrap();
        let failed = error(pending.await.unwrap(), status, code);
        let view = f
            .client
            .call(
                "GET",
                &format!(
                    "{}/operations/{}",
                    f.workspace_path(),
                    failed["operation_id"].as_str().unwrap()
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
        assert_eq!(
            view["result"]["code"],
            if mode == "reservation" {
                "reservation_expired"
            } else {
                code
            }
        );
        assert_eq!(committed(&f, &handle).await, b"");
        let charges:(i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(charges, (4, 0));
        f.invariants().await;
    }
}

fn proof_client(f: &Fixture) -> Client {
    Client {
        app: f.client.app.clone().layer(axum::middleware::from_fn(
            |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                req.headers_mut().insert(
                    "X-Anchorfs-Workflow-Token",
                    "0c".repeat(32).parse().unwrap(),
                );
                req.headers_mut()
                    .insert("X-Anchorfs-Workflow-Epoch", "1".parse().unwrap());
                next.run(req).await
            },
        )),
    }
}
#[tokio::test]
async fn by9_lease_proof_allows_owner_and_expiry_fences_a_bound_upload() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let owner = open(&f, &f.host, &f.host_session, "f", true).await;
    let worker = open(&f, &f.worker, &f.worker_session, "f", false).await;
    sqlx::query("UPDATE workspaces SET workflow_owner_credential_id=$2,workflow_token_hash=$3,workflow_expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1").bind(f.workspace).bind(f.host.id).bind(anchorfs_core::sha256([12;32]).0.as_slice()).execute(&f.pool).await.unwrap();
    let proof = proof_client(&f);
    proof
        .binary(
            "PUT",
            &format!("{}/bytes?offset=0", handle_path(&f, &owner)),
            Some(&f.host),
            Some(&f.host_session),
            Some("owner"),
            &[],
            Body::from("safe"),
        )
        .await
        .status(200);
    sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=1 WHERE id=$1")
        .bind(Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap())
        .execute(&f.pool)
        .await
        .unwrap();
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
    let mut state = AppState::production((*f.config).clone(), f.pool.clone(), f.backend.service());
    state.faults = faults.clone();
    let mut f = f;
    f.client = Client {
        app: build_router(state),
    };
    let proof = proof_client(&f);
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!("{}/bytes?offset=0", handle_path(&f, &worker));
    let pending = tokio::spawn(async move {
        proof
            .binary(
                "PUT",
                &path,
                Some(&actor),
                Some(&session),
                Some("bound"),
                &[],
                Body::from("lost"),
            )
            .await
    });
    faults.wait().await;
    sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
    release.send(true).unwrap();
    error(pending.await.unwrap(), 409, "fenced");
    assert_eq!(committed(&f, &worker).await, b"safe");
    f.invariants().await;
}
