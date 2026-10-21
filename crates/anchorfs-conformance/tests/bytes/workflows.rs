//! Real workflow grants exercise T19 and spec 07 against PostgreSQL and MinIO.
use super::support::*;
use crate::common::client::lease_client;

async fn grant(f: &Fixture, actor: &Credential, session: &Value, action: &str) -> Value {
    f.client
        .call(
            "POST",
            &format!("{}/workflow-lease{action}", f.workspace_path()),
            Some(actor),
            Some(session),
            Some(&Uuid::new_v4().to_string()),
            None,
        )
        .await
        .status(201)
        .body
}
fn proof(f: &Fixture, g: &Value) -> Client {
    lease_client(
        &f.client,
        g["token"].as_str().unwrap(),
        g["mutation_epoch"].as_i64().unwrap(),
    )
}
async fn bound(f: &Fixture, host: &Credential, worker: &Credential, g: &Value) -> Value {
    f.client
        .call(
            "POST",
            &format!("{}/sessions", f.workspace_path()),
            Some(host),
            None,
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"credential_id":worker.id,"bind_to_workflow_lease":g["token"]})),
        )
        .await
        .status(201)
        .body
}
async fn open_proof(
    f: &Fixture,
    c: &Client,
    actor: &Credential,
    session: &Value,
    create: bool,
) -> Value {
    c.call("POST", &format!("{}/handles", f.workspace_path()), Some(actor),Some(session),Some(&Uuid::new_v4().to_string()),Some(json!({"path":"f","read":true,"write":true,"create":create,"exclusive":false,"truncate":false,"mode":420}))).await.status(201).body
}
async fn append_as(
    f: &Fixture,
    c: &Client,
    actor: &Credential,
    session: &Value,
    handle: &Value,
    key: &str,
) -> RawReply {
    c.binary(
        "POST",
        &format!("{}/append", handle_path(f, handle)),
        Some(actor),
        Some(session),
        Some(key),
        &[],
        Body::from("data"),
    )
    .await
}
async fn rebind_worker(f: &Fixture, token: Option<&str>) -> Value {
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/rebind"),
            Some(&f.host),
            Some(&f.host_session),
            Some(&Uuid::new_v4().to_string()),
            Some(json!({"workflow_lease_token":token})),
        )
        .await
        .status(200)
        .body
}
async fn failed_charge(f: &Fixture, failed: &Value, code: &str) {
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
    assert_eq!(receipt["result"]["code"], code);
    let charges:(i64,i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved,(SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1) FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(charges, (4, 0, 1));
}
#[tokio::test]
async fn t19_real_acquire_fences_inflight_upload_and_bound_worker_publishes() {
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
    let mut f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), false).await;
    let h = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let pending = append_task(&f, &h, "inflight", b"data");
    faults.wait().await;
    let g = grant(&f, &f.host, &f.host_session, "").await;
    let c = proof(&f, &g);
    release.send(true).unwrap();
    let failed = error(pending.await.unwrap(), 409, "workspace_busy");
    failed_charge(&f, &failed, "workspace_busy").await;
    assert_eq!(committed(&f, &h).await, b"");
    f.worker_session = rebind_worker(&f, g["token"].as_str()).await;
    append_as(&f, &c, &f.worker, &f.worker_session, &h, "bound")
        .await
        .status(200);
    let host_handle = open_read(&f, &f.host, &f.host_session).await;
    assert_eq!(
        read_as(&f, &host_handle, &f.host, &f.host_session, 0, 4096, None)
            .await
            .status(200)
            .bytes
            .as_ref(),
        b"data"
    );
    let unbound = f.session_for(&f.worker).await;
    let uh = open_read(&f, &f.worker, &unbound).await;
    error(
        append_as(&f, &f.client, &f.worker, &unbound, &uh, "unbound").await,
        409,
        "workspace_busy",
    );
    c.call(
        "DELETE",
        &format!("{}/workflow-lease", f.workspace_path()),
        Some(&f.host),
        Some(&f.host_session),
        Some("release"),
        None,
    )
    .await
    .status(200);
    error(
        append_as(&f, &c, &f.worker, &f.worker_session, &h, "released").await,
        409,
        "fenced",
    );
    error(read(&f, &h, 0, 4096, None).await, 409, "fenced");
    f.worker_session = rebind_worker(&f, None).await;
    append(&f, &h, "ordinary", b"next").await.status(200);
    assert_eq!(committed(&f, &h).await, b"datanext");
    f.invariants().await;
}
#[tokio::test]
async fn takeover_fences_old_lease_upload_and_successor_publishes() {
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
    let f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), false).await;
    let g = grant(&f, &f.host, &f.host_session, "").await;
    let c = proof(&f, &g);
    let w = bound(&f, &f.host, &f.worker, &g).await;
    let h = open_proof(&f, &c, &f.worker, &w, true).await;
    let client = proof(&f, &g);
    let actor = f.worker.clone();
    let session = w.clone();
    let path = format!("{}/append", handle_path(&f, &h));
    let pending = tokio::spawn(async move {
        client
            .binary(
                "POST",
                &path,
                Some(&actor),
                Some(&session),
                Some("inflight"),
                &[],
                Body::from("data"),
            )
            .await
    });
    faults.wait().await;
    sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
    let b = f.credential("host", "write", 1000, 1000).await;
    let bs = f.session_for(&b).await;
    let g2 = grant(&f, &b, &bs, "/takeover").await;
    assert_eq!(
        g2["mutation_epoch"].as_i64().unwrap(),
        g["mutation_epoch"].as_i64().unwrap() + 1
    );
    release.send(true).unwrap();
    let failed = error(pending.await.unwrap(), 409, "fenced");
    failed_charge(&f, &failed, "fenced").await;
    assert_eq!(committed(&f, &h).await, b"");
    let worker2 = f.credential("worker", "write", 1000, 1000).await;
    let w2 = bound(&f, &b, &worker2, &g2).await;
    let c2 = proof(&f, &g2);
    let h2 = open_proof(&f, &c2, &worker2, &w2, false).await;
    append_as(&f, &c2, &worker2, &w2, &h2, "successor")
        .await
        .status(200);
    error(
        append_as(&f, &c, &f.worker, &w, &h, "stale").await,
        409,
        "fenced",
    );
    error(
        read_as(&f, &h, &f.worker, &w, 0, 4096, None).await,
        409,
        "fenced",
    );
    let info = f
        .client
        .call("GET", &f.workspace_path(), Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(
        info["workflow"],
        json!({"owner_credential_id":b.id,"mutation_epoch":g2["mutation_epoch"],"expires_at":g2["expires_at"],"active":true})
    );
    assert_eq!(committed(&f, &h2).await, b"data");
    f.invariants().await;
}
#[tokio::test]
async fn spec07_revoked_worker_cannot_reenter_while_host_and_successor_continue() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let g = grant(&f, &f.host, &f.host_session, "").await;
    let c = proof(&f, &g);
    let w = bound(&f, &f.host, &f.worker, &g).await;
    let h = open_proof(&f, &c, &f.worker, &w, true).await;
    append_as(&f, &c, &f.worker, &w, &h, "before")
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &format!("/v1/credentials/{}", f.worker.id),
            Some(&f.admin),
            None,
            Some("revoke"),
            None,
        )
        .await
        .status(200);
    for (path, body) in [
        (
            format!("{}/sessions", f.workspace_path()),
            Some(json!({"credential_id":f.worker.id})),
        ),
        (f.session_path(&w, "/rebind"), Some(json!({}))),
        (format!("{}/workflow-lease", f.workspace_path()), None),
    ] {
        f.client
            .call(
                "POST",
                &path,
                Some(&f.worker),
                Some(&w),
                Some(&Uuid::new_v4().to_string()),
                body,
            )
            .await
            .error(401, "unauthenticated");
    }
    error(
        append_as(&f, &c, &f.worker, &w, &h, "revoked").await,
        401,
        "unauthenticated",
    );
    c.call(
        "POST",
        &format!("{}/workflow-lease/renew", f.workspace_path()),
        Some(&f.host),
        Some(&f.host_session),
        Some("renew"),
        None,
    )
    .await
    .status(200);
    let worker2 = f.credential("worker", "write", 1000, 1000).await;
    let w2 = bound(&f, &f.host, &worker2, &g).await;
    let h2 = open_proof(&f, &c, &worker2, &w2, false).await;
    append_as(&f, &c, &worker2, &w2, &h2, "successor")
        .await
        .status(200);
    let host_handle = open_read(&f, &f.host, &f.host_session).await;
    assert_eq!(
        read_as(&f, &host_handle, &f.host, &f.host_session, 0, 4096, None)
            .await
            .status(200)
            .bytes
            .as_ref(),
        b"datadata"
    );
    let worker3 = f.credential("worker", "write", 1000, 1000).await;
    let w3 = bound(&f, &f.host, &worker3, &g).await;
    for action in ["", "/takeover", "/renew", "release"] {
        let client = if ["/renew", "release"].contains(&action) {
            proof(&f, &g)
        } else {
            Client {
                app: f.client.app.clone(),
            }
        };
        client
            .call(
                if action == "release" {
                    "DELETE"
                } else {
                    "POST"
                },
                &format!(
                    "{}/workflow-lease{}",
                    f.workspace_path(),
                    if action == "release" { "" } else { action }
                ),
                Some(&worker3),
                Some(&w3),
                Some(&Uuid::new_v4().to_string()),
                None,
            )
            .await
            .error(403, "forbidden");
    }
    f.client
        .call(
            "POST",
            &f.session_path(&w3, "/rebind"),
            Some(&worker3),
            Some(&w3),
            Some("worker-rebind"),
            Some(json!({})),
        )
        .await
        .error(403, "forbidden");
    f.invariants().await;
}

async fn open_read(f: &Fixture, actor: &Credential, session: &Value) -> Value {
    f.client.call("POST", &format!("{}/handles", f.workspace_path()), Some(actor), Some(session), Some(&Uuid::new_v4().to_string()), Some(json!({"path":"f","read":true,"write":false,"create":false,"exclusive":false,"truncate":false}))).await.status(201).body
}
