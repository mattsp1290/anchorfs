use crate::common::fixture::*;
fn proof_client(client: &Client, epoch: i64) -> Client {
    Client {
        app: client.app.clone().layer(axum::middleware::from_fn(
            move |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                req.headers_mut().insert(
                    "X-Anchorfs-Workflow-Token",
                    "0c".repeat(32).parse().unwrap(),
                );
                req.headers_mut().insert(
                    "X-Anchorfs-Workflow-Epoch",
                    epoch.to_string().parse().unwrap(),
                );
                next.run(req).await
            },
        )),
    }
}
#[tokio::test]
async fn ns12_workflow_owner_binding_replay_and_expiry() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    let path = format!("{prefix}/directories");
    let body = json!({"path":"owner","mode":493});
    let opened=f.client.call("POST",&format!("{prefix}/handles"),Some(&f.worker),Some(&f.worker_session),Some("open"),Some(json!({"path":"f","read":true,"write":true,"create":true,"exclusive":false,"truncate":false,"mode":384}))).await.status(201).body;
    sqlx::query("UPDATE workspaces SET workflow_owner_credential_id=$2,workflow_token_hash=$3,workflow_expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1").bind(f.workspace).bind(f.host.id).bind(anchorfs_core::sha256([12;32]).0.to_vec()).execute(&f.pool).await.unwrap();
    f.client
        .call(
            "POST",
            &path,
            Some(&f.host),
            Some(&f.host_session),
            Some("owner"),
            Some(body.clone()),
        )
        .await
        .error(409, "workspace_busy");
    let proof = proof_client(&f.client, 1);
    let committed = proof
        .call(
            "POST",
            &path,
            Some(&f.host),
            Some(&f.host_session),
            Some("owner"),
            Some(body.clone()),
        )
        .await
        .status(200)
        .body;
    proof
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("unbound"),
            Some(json!({"path":"unbound","mode":493})),
        )
        .await
        .error(409, "workspace_busy");
    let id = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=1 WHERE id=$1")
        .bind(id)
        .execute(&f.pool)
        .await
        .unwrap();
    proof
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("bound"),
            Some(json!({"path":"bound","mode":493})),
        )
        .await
        .status(200);
    proof_client(&f.client, 2)
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("stale"),
            Some(json!({"path":"stale","mode":493})),
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &path,
            Some(&f.host),
            Some(&f.host_session),
            Some("owner"),
            Some(body),
        )
        .await
        .error(409, "workspace_busy");
    let receipt = f
        .client
        .call(
            "GET",
            &format!("{prefix}/operations?idempotency_key=owner"),
            Some(&f.host),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(receipt["state"], "committed");
    assert_eq!(receipt["result"], committed);
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200);
    let handle = format!("{prefix}/handles/{}", opened["handle_id"].as_str().unwrap());
    f.client
        .call(
            "POST",
            &format!("{handle}/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("renew"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &handle,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close"),
            None,
        )
        .await
        .status(200);
    sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
    proof
        .call(
            "POST",
            &path,
            Some(&f.host),
            Some(&f.host_session),
            Some("late-proof"),
            Some(json!({"path":"late","mode":493})),
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("late-bound"),
            Some(json!({"path":"late","mode":493})),
        )
        .await
        .error(409, "fenced");
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200);
    f.invariants().await;
}

#[tokio::test]
async fn workflow_expiry_after_publication_wait_rolls_back_owner_and_bound_worker() {
    for (bound_worker, writable_open) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let f = Fixture::new().await;
        let prefix = f.workspace_path();
        if writable_open {
            let grant=f.client.call("POST",&format!("{prefix}/handles"),Some(&f.host),Some(&f.host_session),Some("setup-file"),Some(json!({"path":"existing","read":true,"write":true,"create":true,"exclusive":false,"truncate":false,"mode":384}))).await.status(201).body;
            f.client
                .call(
                    "DELETE",
                    &format!("{prefix}/handles/{}", grant["handle_id"].as_str().unwrap()),
                    Some(&f.host),
                    Some(&f.host_session),
                    Some("setup-close"),
                    None,
                )
                .await
                .status(200);
        }
        let handles_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM handles WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        let session_id = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
        if bound_worker {
            sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=1 WHERE id=$1")
                .bind(session_id)
                .execute(&f.pool)
                .await
                .unwrap();
        }
        let expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE workspaces SET workflow_owner_credential_id=$2,workflow_token_hash=$3,workflow_expires_at=clock_timestamp()+interval '1500 milliseconds' WHERE id=$1 RETURNING workflow_expires_at").bind(f.workspace).bind(f.host.id).bind(anchorfs_core::sha256([12;32]).0.to_vec()).fetch_one(&f.pool).await.unwrap();
        let before:(i64,i64,i64)=sqlx::query_as("SELECT n.namespace_generation,q.logical_inodes_used,i.generation FROM namespace_locks n JOIN workspace_quotas q USING(workspace_id) JOIN workspaces w ON w.id=n.workspace_id JOIN inodes i ON i.workspace_id=w.id AND i.id=w.root_inode_id WHERE w.id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        let mut blocker = f.pool.begin().await.unwrap();
        if writable_open {
            let actor_session = if bound_worker {
                &f.worker_session
            } else {
                &f.host_session
            };
            anchorfs_persistence::handles::lock_admission(
                &mut blocker,
                anchorfs_core::SessionId(
                    Uuid::parse_str(actor_session["session_id"].as_str().unwrap()).unwrap(),
                ),
            )
            .await
            .unwrap();
        } else {
            sqlx::query(
                "SELECT workspace_id FROM namespace_locks WHERE workspace_id=$1 FOR UPDATE",
            )
            .bind(f.workspace)
            .execute(&mut *blocker)
            .await
            .unwrap();
        }
        let client = proof_client(&f.client, 1);
        let actor = if bound_worker {
            f.worker.clone()
        } else {
            f.host.clone()
        };
        let session = if bound_worker {
            f.worker_session.clone()
        } else {
            f.host_session.clone()
        };
        let url = if writable_open {
            format!("{prefix}/handles")
        } else {
            format!("{prefix}/directories")
        };
        let body = if writable_open {
            json!({"path":"existing","read":false,"write":true,"create":false,"exclusive":false,"truncate":false})
        } else {
            json!({"path":"late","mode":493})
        };
        let request = tokio::spawn(async move {
            client
                .call(
                    "POST",
                    &url,
                    Some(&actor),
                    Some(&session),
                    Some("expired-publication"),
                    Some(body),
                )
                .await
        });
        wait_for_lock(
            &f.pool,
            if writable_open {
                "7002"
            } else {
                "SELECT namespace_generation FROM namespace_locks"
            },
        )
        .await;
        wait_until_expired(&f.pool, expiry).await;
        blocker.commit().await.unwrap();
        request.await.unwrap().error(409, "fenced");
        let after:(i64,i64,i64)=sqlx::query_as("SELECT n.namespace_generation,q.logical_inodes_used,i.generation FROM namespace_locks n JOIN workspace_quotas q USING(workspace_id) JOIN workspaces w ON w.id=n.workspace_id JOIN inodes i ON i.workspace_id=w.id AND i.id=w.root_inode_id WHERE w.id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(after, before);
        let entries: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM directory_entries WHERE workspace_id=$1 AND name='late'",
        )
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(entries, 0);
        let receipts:i64=sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key='expired-publication'").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(receipts, 0);
        let handles_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM handles WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(handles_after, handles_before);
        f.invariants().await;
    }
}

use crate::common::client::{Reply, lease_client};
fn lease_path(f: &Fixture, action: &str) -> String {
    format!("{}/workflow-lease{action}", f.workspace_path())
}
async fn lease(
    f: &Fixture,
    client: &Client,
    actor: &Credential,
    session: &Value,
    action: &str,
    key: &str,
) -> Reply {
    client
        .call(
            if action == "release" {
                "DELETE"
            } else {
                "POST"
            },
            &lease_path(f, if action == "release" { "" } else { action }),
            Some(actor),
            Some(session),
            Some(key),
            None,
        )
        .await
}
async fn acquire(f: &Fixture) -> Value {
    lease(f, &f.client, &f.host, &f.host_session, "", "acquire")
        .await
        .status(201)
        .body
}
fn granted(f: &Fixture, grant: &Value) -> Client {
    lease_client(
        &f.client,
        grant["token"].as_str().unwrap(),
        grant["mutation_epoch"].as_i64().unwrap(),
    )
}
async fn expire(f: &Fixture) {
    sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
}
async fn mkdir(f: &Fixture, c: &Client, actor: &Credential, session: &Value, name: &str) -> Reply {
    c.call(
        "POST",
        &format!("{}/directories", f.workspace_path()),
        Some(actor),
        Some(session),
        Some(name),
        Some(json!({"path":name,"mode":493})),
    )
    .await
}
async fn stat(f: &Fixture, actor: &Credential, session: &Value) -> Reply {
    f.client
        .call(
            "GET",
            &format!("{}/stat", f.workspace_path()),
            Some(actor),
            Some(session),
            None,
            None,
        )
        .await
}
async fn bind(f: &Fixture, actor: &Credential, target: &Credential, token: &str) -> Reply {
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(actor),
            None,
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"credential_id":target.id,"bind_to_workflow_lease":token})),
        )
        .await
}
async fn rebind(
    f: &Fixture,
    actor: &Credential,
    actor_session: &Value,
    target: &Value,
    token: Option<&str>,
) -> Reply {
    f.client
        .call(
            "POST",
            &f.session_path(target, "/rebind"),
            Some(actor),
            Some(actor_session),
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"workflow_lease_token":token})),
        )
        .await
}
async fn workspace_info(f: &Fixture, actor: &Credential) -> Value {
    f.client
        .call("GET", &f.workspace_path(), Some(actor), None, None, None)
        .await
        .status(200)
        .body
}
fn message(reply: Reply, status: u16, code: &str, message: &str) {
    assert_eq!(reply.error(status, code).body["message"], message);
}
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
