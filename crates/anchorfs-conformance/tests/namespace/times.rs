//! Stage 2b explicit timestamps: independent synthetic instants and authority cases.
use crate::common::{client::lease_client, fixture::*};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

async fn set(
    f: &Fixture,
    actor: &Credential,
    session: &Value,
    body: Value,
) -> crate::common::client::Reply {
    f.client
        .call(
            "PATCH",
            &format!("{}/attributes", f.workspace_path()),
            Some(actor),
            Some(session),
            Some(&Uuid::new_v4().to_string()),
            Some(body),
        )
        .await
}
async fn stat(f: &Fixture, path: &str) -> Value {
    f.client
        .call(
            "GET",
            &format!("{}/stat?path={path}", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body
}
fn instant(value: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(value.as_str().unwrap(), &Rfc3339).unwrap()
}

#[tokio::test]
async fn set_attributes_times_owner_admin_and_forbidden() {
    let f = Fixture::new().await;
    f.client.call("POST", &format!("{}/handles", f.workspace_path()), Some(&f.worker), Some(&f.worker_session), Some("file"), Some(json!({"path":"f","create":true,"read":true,"write":true,"exclusive":true,"truncate":false,"mode":420}))).await.status(201);
    let before = stat(&f, "f").await;
    let timestamp = "2024-01-02T12:13:14.123456789+02:00";
    set(
        &f,
        &f.worker,
        &f.worker_session,
        json!({"path":"f","mtime":timestamp}),
    )
    .await
    .status(200);
    let after = stat(&f, "f").await;
    assert_eq!(
        instant(&after["mtime"]),
        OffsetDateTime::parse("2024-01-02T10:13:14.123456Z", &Rfc3339).unwrap()
    );
    assert_eq!(
        after["generation"].as_i64().unwrap(),
        before["generation"].as_i64().unwrap() + 1
    );
    assert!(instant(&after["ctime"]) > instant(&before["ctime"]));
    assert_eq!(after["atime"], before["atime"]);
    let other = f.credential("worker", "write", 1001, 1001).await;
    let session = f.session_for(&other).await;
    set(&f, &other, &session, json!({"path":"f","mtime":timestamp}))
        .await
        .error(403, "forbidden");
    let administrator = f.credential("host", "admin", 1002, 1002).await;
    let admin_session = f.session_for(&administrator).await;
    set(
        &f,
        &administrator,
        &admin_session,
        json!({"path":"f","atime":timestamp,"mtime":timestamp}),
    )
    .await
    .status(200);
    let after_admin = stat(&f, "f").await;
    assert_eq!(after_admin["atime"], after_admin["mtime"]);
    for bad in ["bad", "10000-01-01T00:00:00Z", "0000-01-01T00:00:00Z"] {
        set(
            &f,
            &f.worker,
            &f.worker_session,
            json!({"path":"f","mtime":bad}),
        )
        .await
        .error(400, "invalid_input");
    }
    set(&f, &f.worker, &f.worker_session, json!({"path":"f"}))
        .await
        .error(400, "invalid_input");
    assert_eq!(stat(&f, "f").await, after_admin);
    f.client
        .call(
            "POST",
            &format!("{}/symlinks", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("sym"),
            Some(json!({"path":"sym","target":"f"})),
        )
        .await
        .status(200);
    set(
        &f,
        &f.worker,
        &f.worker_session,
        json!({"path":"sym","mtime":timestamp}),
    )
    .await
    .status(200);
    assert_eq!(stat(&f, "sym").await["mtime"], after_admin["mtime"]);
    f.invariants().await;
}

#[tokio::test]
async fn set_attributes_times_replay_and_lease_gate() {
    let f = Fixture::new().await;
    let url = format!("{}/attributes", f.workspace_path());
    let body = json!({"mtime":"2024-02-03T00:00:00Z"});
    let first = f
        .client
        .call(
            "PATCH",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("times"),
            Some(body.clone()),
        )
        .await
        .status(200)
        .body;
    let replay = f
        .client
        .call(
            "PATCH",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("times"),
            Some(body),
        )
        .await
        .status(200)
        .body;
    assert_eq!(first, replay);
    f.client
        .call(
            "PATCH",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("times"),
            Some(json!({"mtime":"2024-02-04T00:00:00Z"})),
        )
        .await
        .error(409, "idempotency_mismatch");
    let lease = f
        .client
        .call(
            "POST",
            &format!("{}/workflow-lease", f.workspace_path()),
            Some(&f.host),
            Some(&f.host_session),
            Some("lease"),
            Some(json!({})),
        )
        .await
        .status(201)
        .body;
    set(
        &f,
        &f.worker,
        &f.worker_session,
        json!({"atime":"2024-02-03T00:00:00Z"}),
    )
    .await
    .error(409, "workspace_busy");
    let client = lease_client(
        &f.client,
        lease["token"].as_str().unwrap(),
        lease["mutation_epoch"].as_i64().unwrap(),
    );
    client
        .call(
            "DELETE",
            &format!("{}/workflow-lease", f.workspace_path()),
            Some(&f.host),
            Some(&f.host_session),
            Some("release"),
            None,
        )
        .await
        .status(200);
    f.invariants().await;
}
