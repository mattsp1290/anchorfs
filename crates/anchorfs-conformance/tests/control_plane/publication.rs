use super::support::*;

#[tokio::test]
async fn expired_credential_after_receipt_wait_cannot_publish_any_admin_mutation() {
    let f = Fixture::new().await;
    for operation in ["workspace", "credential", "session"] {
        let before:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM workspaces),(SELECT count(*) FROM credentials),(SELECT count(*) FROM client_sessions)").fetch_one(&f.pool).await.unwrap();
        let expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE credentials SET expires_at=clock_timestamp()+interval '1500 milliseconds' WHERE id=$1 RETURNING expires_at").bind(f.admin.id).fetch_one(&f.pool).await.unwrap();
        let mut holder = f.pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE admin_operations IN SHARE MODE")
            .execute(&mut *holder)
            .await
            .unwrap();
        let (path, body) = match operation {
            "workspace" => ("/v1/workspaces".to_string(), workspace_body()),
            "credential" => (
                "/v1/credentials".to_string(),
                json!({"kind":"administrator","role":"admin","uid":1000,"gid":1000,"max_reservation_bytes":4096}),
            ),
            _ => (
                format!("{}/sessions", f.workspace_path()),
                json!({"credential_id":f.worker.id}),
            ),
        };
        let client = Client {
            app: f.client.app.clone(),
        };
        let admin = f.admin.clone();
        let key = format!("late-{operation}");
        let request_key = key.clone();
        let request = tokio::spawn(async move {
            client
                .call(
                    "POST",
                    &path,
                    Some(&admin),
                    None,
                    Some(&request_key),
                    Some(body),
                )
                .await
        });
        wait_for_lock(&f.pool, "INSERT INTO admin_operations").await;
        wait_until_expired(&f.pool, expiry).await;
        holder.commit().await.unwrap();
        request.await.unwrap().error(401, "unauthenticated");
        let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM workspaces),(SELECT count(*) FROM credentials),(SELECT count(*) FROM client_sessions)").fetch_one(&f.pool).await.unwrap();
        assert_eq!(before, after);
        let receipts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM admin_operations WHERE idempotency_key=$1")
                .bind(key)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(receipts, 0);
        sqlx::query("UPDATE credentials SET expires_at=NULL WHERE id=$1")
            .bind(f.admin.id)
            .execute(&f.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn expired_actor_session_after_target_wait_rolls_back_mutation_and_receipt() {
    let f = Fixture::new().await;
    let target = Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap();
    let actor = Uuid::parse_str(f.host_session["session_id"].as_str().unwrap()).unwrap();
    let before: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM client_sessions WHERE id=$1")
            .bind(target)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE client_sessions SET expires_at=clock_timestamp()+interval '1500 milliseconds' WHERE id=$1 RETURNING expires_at").bind(actor).fetch_one(&f.pool).await.unwrap();
    let mut holder = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM client_sessions WHERE id=$1 FOR UPDATE")
        .bind(target)
        .execute(&mut *holder)
        .await
        .unwrap();
    let client = Client {
        app: f.client.app.clone(),
    };
    let host = f.host.clone();
    let session = f.host_session.clone();
    let path = f.session_path(&f.worker_session, "/renew");
    let request = tokio::spawn(async move {
        client
            .call(
                "POST",
                &path,
                Some(&host),
                Some(&session),
                Some("late-session"),
                None,
            )
            .await
    });
    wait_for_lock(&f.pool, "SELECT * FROM client_sessions").await;
    wait_until_expired(&f.pool, expiry).await;
    holder.commit().await.unwrap();
    request.await.unwrap().error(409, "fenced");
    let after: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM client_sessions WHERE id=$1")
            .bind(target)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE idempotency_key='late-session'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(receipts, 0);
}
