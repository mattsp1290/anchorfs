use super::support::*;

#[tokio::test]
async fn cp6_credential_authority_and_secret_hygiene() {
    let f = Fixture::new().await;
    let mut body = credential_body(f.workspace, "worker");
    body["uid"] = json!(2000);
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("foreign-uid"),
            Some(body),
        )
        .await
        .error(403, "forbidden");
    for (field, value) in [
        ("gid", json!(2000)),
        ("max_reservation_bytes", json!(4097)),
        ("kind", json!("host")),
    ] {
        let mut body = credential_body(f.workspace, "worker");
        body[field] = value;
        f.client
            .call(
                "POST",
                "/v1/credentials",
                Some(&f.host),
                None,
                Some(field),
                Some(body),
            )
            .await
            .error(403, "forbidden");
    }
    let body = credential_body(Uuid::new_v4(), "worker");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("missing-workspace"),
            Some(body),
        )
        .await
        .error(404, "not_found");
    let mut body = credential_body(f.workspace, "worker");
    body["role"] = json!("admin");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("bad-role"),
            Some(body),
        )
        .await
        .error(400, "invalid_input");
    let mut body = credential_body(f.workspace, "worker");
    body["expires_at"] = json!("2000-01-01T00:00:00Z");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("past-expiry"),
            Some(body),
        )
        .await
        .error(400, "invalid_input");
    let response = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("fresh-worker"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .status(201);
    assert_eq!(response.headers["Cache-Control"], "no-store");
    let issued = Credential::from_grant(&response.body);
    let receipts: Vec<Value> =
        sqlx::query_scalar("SELECT result FROM admin_operations WHERE result IS NOT NULL")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    for result in receipts {
        assert!(!result.to_string().contains(&issued.secret));
        assert!(!result.to_string().contains(&f.worker.secret));
    }
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.worker),
            None,
            Some("worker-issue"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .error(403, "forbidden");
}

#[tokio::test]
async fn cp9_rotation_revokes_sessions_and_lost_self_rotation_is_recoverable() {
    let f = Fixture::new().await;
    let mint = format!("{}/sessions", f.workspace_path());
    for slot in 1..64 {
        f.client
            .call(
                "POST",
                &mint,
                Some(&f.host),
                None,
                Some(&format!("fill-{slot}")),
                Some(json!({"credential_id":f.worker.id})),
            )
            .await
            .status(201);
    }
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("over-limit"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .error(507, "quota_exceeded");
    let rotate = format!("/v1/credentials/{}/rotate", f.worker.id);
    f.client
        .call(
            "POST",
            &rotate,
            Some(&f.worker),
            None,
            Some("worker-rotate"),
            None,
        )
        .await
        .error(403, "forbidden");
    let issued = f
        .client
        .call(
            "POST",
            &rotate,
            Some(&f.host),
            None,
            Some("rotate-worker"),
            None,
        )
        .await
        .status(200);
    assert_eq!(issued.headers["Cache-Control"], "no-store");
    assert_eq!(issued.body["credential_epoch"], 2);
    let revoked_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NOT NULL",
    )
    .bind(f.worker.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(revoked_sessions, 64);
    let new_worker = Credential::from_grant(&issued.body);
    f.client
        .call(
            "GET",
            &f.workspace_path(),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .error(401, "unauthenticated");
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&new_worker),
            Some(&f.worker_session),
            Some("fenced-session"),
            None,
        )
        .await
        .error(409, "fenced");
    let replay = f
        .client
        .call(
            "POST",
            &rotate,
            Some(&f.host),
            None,
            Some("rotate-worker"),
            None,
        )
        .await
        .status(200)
        .body;
    assert!(replay["secret"].is_null());
    assert_eq!(replay["credential_epoch"], 2);
    let session = f
        .client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.host),
            None,
            Some("post-rotation-mint"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .status(201)
        .body;
    assert_eq!(session["credential_epoch"], 2);
    let self_rotate = format!("/v1/credentials/{}/rotate", f.host.id);
    // Deliberately discard the secret response to model the lost first response.
    f.client
        .call(
            "POST",
            &self_rotate,
            Some(&f.host),
            None,
            Some("self-rotate"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &self_rotate,
            Some(&f.host),
            None,
            Some("self-rotate"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let recovered = f
        .client
        .call(
            "POST",
            &self_rotate,
            Some(&f.admin),
            None,
            Some("recover-host"),
            None,
        )
        .await
        .status(200)
        .body;
    let recovered = Credential::from_grant(&recovered);
    f.client
        .call(
            "GET",
            &f.workspace_path(),
            Some(&recovered),
            None,
            None,
            None,
        )
        .await
        .status(200);
    let stale_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL",
    )
    .bind(f.host.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(stale_count, 0);
}

#[tokio::test]
async fn cp10_revocation_cascades_and_bootstrap_recovers_all_administrators_revoked() {
    let f = Fixture::new().await;
    let revoke = format!("/v1/credentials/{}", f.host.id);
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.host),
            None,
            Some("self-revoke-host"),
            None,
        )
        .await
        .error(403, "forbidden");
    f.client
        .call(
            "DELETE",
            &revoke,
            Some(&f.admin),
            None,
            Some("revoke-host"),
            None,
        )
        .await
        .status(200);
    for credential in [&f.host, &f.worker] {
        f.client
            .call(
                "GET",
                &f.workspace_path(),
                Some(credential),
                None,
                None,
                None,
            )
            .await
            .error(401, "unauthenticated");
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM credentials WHERE id=ANY($1) AND revoked_at IS NOT NULL AND epoch=2",
    )
    .bind(vec![f.host.id, f.worker.id])
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 2);
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM client_sessions WHERE id=$1")
            .bind(Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(revoked);
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(&f.admin),
            None,
            Some("revoked-mint"),
            Some(json!({"credential_id":f.worker.id})),
        )
        .await
        .error(404, "not_found");
    let admin_b=f.client.call("POST","/v1/credentials",Some(&f.admin),None,Some("admin-b"),Some(json!({"kind":"administrator","role":"admin","uid":1000,"gid":1000,"max_reservation_bytes":4096}))).await.status(201).body;
    let b = Credential::from_grant(&admin_b);
    f.client
        .call(
            "DELETE",
            &format!("/v1/credentials/{}", f.admin.id),
            Some(&b),
            None,
            Some("revoke-ancestry"),
            None,
        )
        .await
        .status(200);
    for credential in [&f.admin, &b] {
        f.client
            .call(
                "GET",
                &f.workspace_path(),
                Some(credential),
                None,
                None,
                None,
            )
            .await
            .error(401, "unauthenticated");
    }
    anchorfs_service::bootstrap::create_administrator(&f.pool, 1000, 1000, 4096, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn admin_hosts_are_reachable_and_cannot_delegate_admin() {
    let f = Fixture::new().await;
    let mut body = credential_body(f.workspace, "host");
    body["role"] = json!("admin");
    let grant = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("admin-host"),
            Some(body.clone()),
        )
        .await
        .status(201)
        .body;
    let host = Credential::from_grant(&grant);
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&host),
            None,
            Some("delegate-admin"),
            Some(body),
        )
        .await
        .error(403, "forbidden");
    let mut worker = credential_body(f.workspace, "worker");
    worker["role"] = json!("admin");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("admin-worker"),
            Some(worker),
        )
        .await
        .error(400, "invalid_input");
    f.backend.untouched();
}
