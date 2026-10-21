pub(super) use crate::common::client::{Reply, lease_client};
pub(super) use crate::common::fixture::*;
pub(super) fn lease_path(f: &Fixture, action: &str) -> String {
    format!("{}/workflow-lease{action}", f.workspace_path())
}
pub(super) async fn lease(
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
pub(super) async fn acquire(f: &Fixture) -> Value {
    lease(f, &f.client, &f.host, &f.host_session, "", "acquire")
        .await
        .status(201)
        .body
}
pub(super) fn granted(f: &Fixture, grant: &Value) -> Client {
    lease_client(
        &f.client,
        grant["token"].as_str().unwrap(),
        grant["mutation_epoch"].as_i64().unwrap(),
    )
}
pub(super) async fn expire(f: &Fixture) {
    sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
}
pub(super) async fn mkdir(
    f: &Fixture,
    c: &Client,
    actor: &Credential,
    session: &Value,
    name: &str,
) -> Reply {
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
pub(super) async fn stat(f: &Fixture, actor: &Credential, session: &Value) -> Reply {
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
pub(super) async fn bind(
    f: &Fixture,
    actor: &Credential,
    target: &Credential,
    token: &str,
) -> Reply {
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
pub(super) async fn rebind(
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
pub(super) async fn workspace_info(f: &Fixture, actor: &Credential) -> Value {
    f.client
        .call("GET", &f.workspace_path(), Some(actor), None, None, None)
        .await
        .status(200)
        .body
}
pub(super) fn message(reply: Reply, status: u16, code: &str, message: &str) {
    assert_eq!(reply.error(status, code).body["message"], message);
}
