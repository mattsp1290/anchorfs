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
