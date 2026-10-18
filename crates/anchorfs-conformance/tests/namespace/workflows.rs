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
