use super::support::*;

#[tokio::test]
async fn cp7_cp8_cp11_session_authority_epochs_and_self_fencing() {
    let f = Fixture::new().await;
    let mint = format!("{}/sessions", f.workspace_path());
    let target = json!({"credential_id":f.worker.id});
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.worker),
            None,
            Some("worker-mint"),
            Some(target.clone()),
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.admin),
            None,
            Some("admin-mint"),
            Some(target.clone()),
        )
        .await
        .status(201);
    let mut lease = target.clone();
    lease["bind_to_workflow_lease"] = json!("a".repeat(64));
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("lease"),
            Some(lease),
        )
        .await
        .error(409, "workspace_busy");
    let granted = f
        .client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("mint"),
            Some(target),
        )
        .await
        .status(201)
        .body;
    let now: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let expiry = time::OffsetDateTime::parse(
        granted["expires_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!((expiry - now).whole_seconds() >= 3590 && (expiry - now).whole_seconds() <= 3601);
    let renew = f.session_path(&f.worker_session, "/renew");
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.host_session),
            Some("foreign-session"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let mut stale = f.worker_session.clone();
    stale["session_epoch"] = json!(2);
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&stale),
            Some("bad-epoch"),
            None,
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &f.session_path(&f.host_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew-other"),
            None,
        )
        .await
        .error(403, "forbidden");
    let rebind = f.session_path(&f.worker_session, "/rebind");
    f.client
        .call(
            "POST",
            &rebind,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("worker-rebind"),
            Some(json!({})),
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("lease-rebind"),
            Some(json!({"workflow_lease_token":"a".repeat(64)})),
        )
        .await
        .error(409, "workspace_busy");
    let rebound = f
        .client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(rebound["session_epoch"], 2);
    assert!(rebound["bound_mutation_epoch"].is_null());
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("old-epoch"),
            None,
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.worker),
            Some(&rebound),
            Some("new-epoch"),
            None,
        )
        .await
        .status(200);
    sqlx::query(
        "UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(Uuid::parse_str(rebound["session_id"].as_str().unwrap()).unwrap())
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call(
            "POST",
            &renew,
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-renew"),
            None,
        )
        .await
        .error(409, "fenced");
    let restored = f
        .client
        .call(
            "POST",
            &rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(restored["session_epoch"], 3);
    let revoke = f.session_path(&restored, "");
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.worker),
            Some(&restored),
            Some("self-revoke"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.worker),
            Some(&restored),
            Some("self-revoke"),
            None,
        )
        .await
        .error(409, "fenced");
    let host_rebind = f.session_path(&f.host_session, "/rebind");
    let host_epoch2 = f
        .client
        .call(
            "POST",
            &host_rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("self-rebind"),
            Some(json!({})),
        )
        .await
        .status(200)
        .body;
    f.client
        .call(
            "POST",
            &host_rebind,
            Some(&f.host),
            Some(&f.host_session),
            Some("self-rebind"),
            Some(json!({})),
        )
        .await
        .error(409, "fenced");
    assert_eq!(host_epoch2["session_epoch"], 2);
    // Failed business mutations leave no receipt.
    let rejected: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE idempotency_key=ANY($1)")
            .bind(vec!["renew-other", "worker-rebind", "expired-renew"])
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(rejected, 0);
}

#[tokio::test]
async fn expired_rebind_respects_capacity_and_active_rebind_keeps_its_slot() {
    let f = Fixture::new().await;
    let expired = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    sqlx::query(
        "UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expired)
    .execute(&f.pool)
    .await
    .unwrap();
    let mint = format!("{}/sessions", f.workspace_path());
    let mut active = Value::Null;
    for slot in 0..64 {
        active = f
            .client
            .call(
                "POST",
                &mint,
                Some(&f.host),
                None,
                Some(&format!("replacement-{slot}")),
                Some(json!({"credential_id":f.worker.id})),
            )
            .await
            .status(201)
            .body;
    }
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some("expired-over-capacity"),
            Some(json!({})),
        )
        .await
        .error(507, "quota_exceeded");
    f.client
        .call(
            "POST",
            &f.session_path(&active, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some("active-at-capacity"),
            Some(json!({})),
        )
        .await
        .status(200);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp()").bind(f.worker.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 64);
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE idempotency_key='expired-over-capacity'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0);
}
