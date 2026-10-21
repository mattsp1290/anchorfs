use super::support::*;

#[tokio::test]
async fn bound_sessions_fail_closed_until_rebind() {
    let f = Fixture::new().await;
    let g = acquire(&f).await;
    let token = g["token"].as_str().unwrap();
    let c = granted(&f, &g);
    let w = bind(&f, &f.host, &f.worker, token).await.status(201).body;
    assert_eq!(w["bound_mutation_epoch"], g["mutation_epoch"]);
    mkdir(&f, &c, &f.worker, &w, "bound").await.status(200);
    mkdir(&f, &f.client, &f.worker, &w, "missing")
        .await
        .error(409, "fenced");
    mkdir(
        &f,
        &lease_client(&f.client, &"ff".repeat(32), 2),
        &f.worker,
        &w,
        "wrong",
    )
    .await
    .error(409, "fenced");
    mkdir(&f, &f.client, &f.worker, &f.worker_session, "unbound")
        .await
        .error(409, "workspace_busy");
    bind(&f, &f.worker, &f.worker, token)
        .await
        .error(403, "forbidden");
    rebind(&f, &f.worker, &w, &w, Some(token))
        .await
        .error(403, "forbidden");
    lease(&f, &c, &f.host, &f.host_session, "release", "release")
        .await
        .status(200);
    mkdir(&f, &c, &f.worker, &w, "released")
        .await
        .error(409, "fenced");
    stat(&f, &f.worker, &w).await.error(409, "fenced");
    message(
        rebind(&f, &f.host, &f.host_session, &w, Some(token)).await,
        409,
        "workspace_busy",
        "lease_absent",
    );
    let g2 = lease(&f, &f.client, &f.host, &f.host_session, "", "again")
        .await
        .status(201)
        .body;
    let w2 = rebind(&f, &f.host, &f.host_session, &w, g2["token"].as_str())
        .await
        .status(200)
        .body;
    assert_eq!(
        w2["session_epoch"].as_i64().unwrap(),
        w["session_epoch"].as_i64().unwrap() + 1
    );
    assert_eq!(w2["bound_mutation_epoch"], g2["mutation_epoch"]);
    mkdir(&f, &granted(&f, &g2), &f.worker, &w2, "rebound")
        .await
        .status(200);
    let w3 = rebind(&f, &f.host, &f.host_session, &w2, None)
        .await
        .status(200)
        .body;
    assert!(w3["bound_mutation_epoch"].is_null());
    lease(
        &f,
        &granted(&f, &g2),
        &f.host,
        &f.host_session,
        "release",
        "release2",
    )
    .await
    .status(200);
    mkdir(&f, &f.client, &f.worker, &w3, "ordinary")
        .await
        .status(200);
    let g3 = lease(&f, &f.client, &f.host, &f.host_session, "", "third")
        .await
        .status(201)
        .body;
    message(
        bind(&f, &f.host, &f.worker, &"00".repeat(32)).await,
        409,
        "fenced",
        "workflow_proof_mismatch",
    );
    let b = f.credential("host", "write", 1000, 1000).await;
    bind(&f, &b, &f.worker, g3["token"].as_str().unwrap())
        .await
        .error(403, "forbidden");
    let values: Vec<Value> = sqlx::query_scalar(
        "SELECT result FROM operations WHERE result IS NOT NULL AND workspace_id=$1",
    )
    .bind(f.workspace)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    for value in values {
        for grant in [&g, &g2, &g3] {
            assert!(!value.to_string().contains(grant["token"].as_str().unwrap()));
        }
    }
    f.invariants().await;
}
#[tokio::test]
async fn lease_routes_reject_workers_read_hosts_and_bound_actors() {
    let f = Fixture::new().await;
    bind(&f, &f.worker, &f.worker, &"00".repeat(32))
        .await
        .error(403, "forbidden");
    let read = f.credential("host", "read", 1000, 1000).await;
    let rs = f.session_for(&read).await;
    let mut initial = None;
    for live in [false, true] {
        if live {
            initial = Some(acquire(&f).await);
        }
        for action in ["", "/takeover", "/renew", "release"] {
            let c = if ["/renew", "release"].contains(&action) {
                lease_client(&f.client, &"00".repeat(32), 2)
            } else {
                Client {
                    app: f.client.app.clone(),
                }
            };
            lease(
                &f,
                &c,
                &f.worker,
                &f.worker_session,
                action,
                &Uuid::new_v4().to_string(),
            )
            .await
            .error(403, "forbidden");
        }
    }
    lease(&f, &f.client, &read, &rs, "", "read")
        .await
        .error(403, "forbidden");
    let g = initial.unwrap();
    let bound = rebind(
        &f,
        &f.host,
        &f.host_session,
        &f.host_session,
        g["token"].as_str(),
    )
    .await
    .status(200)
    .body;
    for key in ["bound", "acquire"] {
        message(
            lease(&f, &f.client, &f.host, &bound, "", key).await,
            409,
            "fenced",
            "session_bound",
        );
    }
    message(
        lease(&f, &granted(&f, &g), &f.host, &bound, "", "headers").await,
        400,
        "invalid_input",
        "workflow_proof_not_applicable",
    );
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM workspaces WHERE id=$1 FOR SHARE")
        .bind(f.workspace)
        .execute(&mut *holder)
        .await
        .unwrap();
    let bad = Credential {
        id: f.host.id,
        secret: "00".repeat(32),
    };
    // A foreign-workspace host credential must exist in this same database.
    let created = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("foreign-workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let foreign=Credential::from_grant(&f.client.call("POST","/v1/credentials",Some(&f.admin),None,Some("foreign-host"),Some(json!({"workspace_id":created["workspace"]["workspace_id"],"kind":"host","role":"write","uid":1000,"gid":1000,"max_reservation_bytes":4096}))).await.status(201).body);
    for (actor, session, status, code) in [
        (&bad, &bound, 401, "unauthenticated"),
        (&f.worker, &f.worker_session, 403, "forbidden"),
        (&read, &rs, 403, "forbidden"),
        (&foreign, &f.host_session, 404, "not_found"),
    ] {
        tokio::time::timeout(
            Duration::from_millis(500),
            lease(&f, &f.client, actor, session, "", "pre-lock"),
        )
        .await
        .expect("ineligible call waited for exclusive lock")
        .error(status, code);
    }
    let actor = f.host.clone();
    let session = f.session_for(&actor).await;
    let c = Client {
        app: f.client.app.clone(),
    };
    let path = lease_path(&f, "");
    let control = tokio::spawn(async move {
        c.call(
            "POST",
            &path,
            Some(&actor),
            Some(&session),
            Some("control-wait"),
            None,
        )
        .await
    });
    wait_for_lock(&f.pool, "FROM workspaces WHERE id=$1 FOR UPDATE").await;
    assert!(!control.is_finished());
    holder.commit().await.unwrap();
    message(
        control.await.unwrap(),
        409,
        "workspace_busy",
        "lease_active",
    );
    f.invariants().await;
}
