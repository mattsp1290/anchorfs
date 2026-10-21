use super::support::*;

#[tokio::test]
async fn lease_lifecycle_receipts_and_replay() {
    let f = Fixture::new().await;
    let first = lease(&f, &f.client, &f.host, &f.host_session, "", "acquire")
        .await
        .status(201);
    assert_eq!(first.headers["cache-control"], "no-store");
    let g = first.body;
    let token = g["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(g["mutation_epoch"], 2);
    let mut stored = g.clone();
    stored["token"] = Value::Null;
    assert_eq!(
        lease(&f, &f.client, &f.host, &f.host_session, "", "acquire")
            .await
            .status(201)
            .body,
        stored
    );
    message(
        lease(&f, &f.client, &f.host, &f.host_session, "", "busy").await,
        409,
        "workspace_busy",
        "lease_active",
    );
    let c = granted(&f, &g);
    let renewed = lease(&f, &c, &f.host, &f.host_session, "/renew", "renew")
        .await
        .status(200)
        .body;
    assert!(renewed["token"].is_null());
    assert_eq!(renewed["mutation_epoch"], g["mutation_epoch"]);
    let t = |v: &Value| {
        time::OffsetDateTime::parse(
            v["expires_at"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap()
    };
    assert!(t(&renewed) > t(&g));
    assert_eq!(
        lease(&f, &c, &f.host, &f.host_session, "/renew", "renew")
            .await
            .status(200)
            .body,
        renewed
    );
    let wrong = lease_client(&f.client, token, 3);
    lease(&f, &wrong, &f.host, &f.host_session, "/renew", "renew")
        .await
        .error(409, "idempotency_mismatch");
    message(
        lease(&f, &wrong, &f.host, &f.host_session, "/renew", "wrong").await,
        409,
        "fenced",
        "workflow_proof_mismatch",
    );
    let b = f.credential("host", "write", 1000, 1000).await;
    let bs = f.session_for(&b).await;
    message(
        lease(&f, &f.client, &b, &bs, "/takeover", "takeover").await,
        409,
        "workspace_busy",
        "lease_active",
    );
    let release = lease(&f, &c, &f.host, &f.host_session, "release", "release")
        .await
        .status(200)
        .body;
    assert_eq!(
        lease(&f, &c, &f.host, &f.host_session, "release", "release")
            .await
            .status(200)
            .body,
        release
    );
    assert_eq!(workspace_info(&f, &f.host).await["mutation_epoch"], 3);
    for (key, expected) in [
        ("acquire", stored),
        ("renew", renewed),
        ("release", release),
    ] {
        let receipt = f
            .client
            .call(
                "GET",
                &format!("{}/operations?idempotency_key={key}", f.workspace_path()),
                Some(&f.host),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body;
        assert_eq!(receipt["state"], "committed");
        assert_eq!(receipt["result"], expected);
        assert!(!receipt.to_string().contains(token));
    }
    f.invariants().await;
}
#[tokio::test]
async fn expired_lease_requires_takeover() {
    let f = Fixture::new().await;
    let g = acquire(&f).await;
    let c = granted(&f, &g);
    let w = bind(&f, &f.host, &f.worker, g["token"].as_str().unwrap())
        .await
        .status(201)
        .body;
    mkdir(&f, &c, &f.worker, &w, "before").await.status(200);
    expire(&f).await;
    mkdir(&f, &c, &f.worker, &w, "expired")
        .await
        .error(409, "fenced");
    stat(&f, &f.worker, &w).await.status(200);
    for action in ["/renew", "release"] {
        message(
            lease(
                &f,
                &c,
                &f.host,
                &f.host_session,
                action,
                &Uuid::new_v4().to_string(),
            )
            .await,
            409,
            "fenced",
            "lease_expired",
        );
    }
    message(
        lease(&f, &f.client, &f.host, &f.host_session, "", "new").await,
        409,
        "workspace_busy",
        "lease_expired",
    );
    let b = f.credential("host", "write", 1000, 1000).await;
    let bs = f.session_for(&b).await;
    for action in ["/renew", "release"] {
        lease(&f, &c, &b, &bs, action, &Uuid::new_v4().to_string())
            .await
            .error(403, "forbidden");
    }
    let g2 = lease(&f, &f.client, &b, &bs, "/takeover", "takeover")
        .await
        .status(201)
        .body;
    assert_eq!(
        g2["mutation_epoch"].as_i64().unwrap(),
        g["mutation_epoch"].as_i64().unwrap() + 1
    );
    mkdir(&f, &c, &f.host, &f.host_session, "displaced")
        .await
        .error(409, "workspace_busy");
    lease(&f, &c, &f.host, &f.host_session, "/renew", "old")
        .await
        .error(403, "forbidden");
    mkdir(&f, &granted(&f, &g2), &b, &bs, "successor")
        .await
        .status(200);
    mkdir(&f, &c, &f.worker, &w, "stale")
        .await
        .error(409, "fenced");
    stat(&f, &f.worker, &w).await.error(409, "fenced");
    assert_eq!(
        workspace_info(&f, &f.host).await["workflow"]["owner_credential_id"],
        b.id.to_string()
    );
    f.invariants().await;
}
#[tokio::test]
async fn lease_expires_by_timer() {
    let f = Fixture::with(workspace_body(), |c| {
        c.workflow_lease_ttl = Duration::from_secs(3)
    })
    .await;
    let g = acquire(&f).await;
    let expiry = time::OffsetDateTime::parse(
        g["expires_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    wait_until_expired(&f.pool, expiry).await;
    message(
        lease(
            &f,
            &granted(&f, &g),
            &f.host,
            &f.host_session,
            "/renew",
            "late",
        )
        .await,
        409,
        "fenced",
        "lease_expired",
    );
    assert_eq!(
        workspace_info(&f, &f.host).await["workflow"]["active"],
        false
    );
    f.invariants().await;
}
#[tokio::test]
async fn workspace_reports_lease_status() {
    let f = Fixture::new().await;
    assert!(workspace_info(&f, &f.host).await["workflow"].is_null());
    let g = acquire(&f).await;
    let info = workspace_info(&f, &f.worker).await;
    assert_eq!(
        info["workflow"],
        json!({"owner_credential_id":f.host.id,"mutation_epoch":g["mutation_epoch"],"expires_at":g["expires_at"],"active":true})
    );
    assert!(!info.to_string().contains("token"));
    expire(&f).await;
    assert_eq!(
        workspace_info(&f, &f.host).await["workflow"]["active"],
        false
    );
    let g2 = lease(&f, &f.client, &f.host, &f.host_session, "/takeover", "take")
        .await
        .status(201)
        .body;
    lease(
        &f,
        &granted(&f, &g2),
        &f.host,
        &f.host_session,
        "release",
        "release",
    )
    .await
    .status(200);
    assert!(workspace_info(&f, &f.worker).await["workflow"].is_null());
    f.invariants().await;
}
