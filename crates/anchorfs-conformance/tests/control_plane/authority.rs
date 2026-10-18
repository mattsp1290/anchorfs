use super::support::*;

#[tokio::test]
async fn cp2_cp3_authentication_shapes_and_workspace_isolation() {
    let f = Fixture::new().await;
    let path = f.workspace_path();
    let root = f
        .client
        .call("GET", &path, Some(&f.worker), None, None, None)
        .await
        .status(200)
        .body["root"]
        .clone();
    assert_eq!(root["kind"], "directory");
    assert_eq!(root["mode"], 0o755);
    assert_eq!(root["uid"], 1000);
    assert_eq!(root["gid"], 1000);
    assert_eq!(root["nlink"], 2);
    assert_eq!(root["generation"], 1);
    for path in [
        path.clone(),
        format!("/v1/workspaces/{}", Uuid::new_v4()),
        format!("/v1/admin-operations/{}", Uuid::new_v4()),
    ] {
        let reply = f
            .client
            .call("GET", &path, None, None, None, None)
            .await
            .error(401, "unauthenticated");
        assert_eq!(reply.headers["WWW-Authenticate"], "Bearer");
        let wrong = Credential {
            id: f.admin.id,
            secret: "0".repeat(64),
        };
        let reply = f
            .client
            .call("GET", &path, Some(&wrong), None, None, None)
            .await
            .error(401, "unauthenticated");
        assert_eq!(
            reply.headers["WWW-Authenticate"],
            "Bearer error=\"invalid_token\""
        );
    }
    f.client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.worker),
            None,
            Some("forbidden"),
            Some(workspace_body()),
        )
        .await
        .error(403, "forbidden");
    let other = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("other-workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let other = format!(
        "/v1/workspaces/{}",
        other["workspace"]["workspace_id"].as_str().unwrap()
    );
    f.client
        .call("GET", &other, Some(&f.host), None, None, None)
        .await
        .error(404, "not_found");
    f.client
        .call("GET", &other, Some(&f.admin), None, None, None)
        .await
        .status(200);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/workspaces")
        .header("Authorization", "Basic invalid")
        .header("content-type", "application/json")
        .body(Body::from("{"))
        .unwrap();
    f.client.raw(request).await.error(401, "unauthenticated");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/workspaces")
        .header(
            "Authorization",
            format!("Bearer {}.{}", Uuid::new_v4(), "a".repeat(64)),
        )
        .header("Idempotency-Key", "malformed")
        .header("content-type", "application/json")
        .body(Body::from("{"))
        .unwrap();
    f.client.raw(request).await.error(400, "invalid_input");
    f.client
        .call("GET", "/v1/workspaces/bad", None, None, None, None)
        .await
        .error(400, "invalid_input");
    sqlx::query(
        "UPDATE credentials SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(f.worker.id)
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call("GET", &path, Some(&f.worker), None, None, None)
        .await
        .error(401, "unauthenticated");
}

#[tokio::test]
async fn cp2_cp3_cp6_cp8_matrix_edges() {
    let f = Fixture::new().await;
    for invalid in [
        "Bearer x".to_string(),
        format!("Bearer {}.{}", f.admin.id, "a".repeat(63)),
        format!("Bearer {}.{}", Uuid::new_v4(), "a".repeat(64)),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/workspaces")
            .header("Authorization", invalid)
            .header("Idempotency-Key", "bad-bearer")
            .header("content-type", "application/json")
            .body(Body::from(workspace_body().to_string()))
            .unwrap();
        let reply = f.client.raw(request).await.error(401, "unauthenticated");
        assert!(!reply.body.to_string().contains(&f.admin.secret));
    }
    let wrong = Credential {
        id: f.admin.id,
        secret: "0".repeat(64),
    };
    f.client
        .call(
            "POST",
            &format!("/v1/credentials/{}/rotate", Uuid::new_v4()),
            Some(&wrong),
            None,
            Some("unknown-target"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    for (field, value) in [
        ("root_mode", json!(2541)),
        ("logical_bytes_limit", json!(0)),
    ] {
        let mut body = workspace_body();
        body[field] = value;
        f.client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&f.admin),
                None,
                Some(field),
                Some(body),
            )
            .await
            .error(400, "invalid_input");
    }
    let missing_session = json!({"session_id":Uuid::new_v4(),"session_epoch":1});
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&missing_session),
            Some("unknown-session"),
            None,
        )
        .await
        .error(401, "unauthenticated");
    let admin_owned = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("admin-owned"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .status(201)
        .body;
    let worker = Credential::from_grant(&admin_owned);
    f.client
        .call(
            "POST",
            &format!("/v1/credentials/{}/rotate", worker.id),
            Some(&f.host),
            None,
            Some("not-issuer"),
            None,
        )
        .await
        .error(403, "forbidden");
    let renewed = f
        .client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("private-receipt"),
            None,
        )
        .await
        .status(200)
        .body;
    f.client
        .call(
            "GET",
            &format!(
                "{}/operations/{}",
                f.workspace_path(),
                renewed["operation_id"].as_str().unwrap()
            ),
            Some(&worker),
            None,
            None,
            None,
        )
        .await
        .error(404, "not_found");
    f.client
        .call(
            "DELETE",
            &f.session_path(&f.worker_session, ""),
            Some(&f.host),
            Some(&f.host_session),
            Some("host-revoke-session"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &f.session_path(&f.worker_session, "/renew"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("revoked-actor"),
            None,
        )
        .await
        .error(409, "fenced");
    let other = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("other"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let other = Uuid::parse_str(other["workspace"]["workspace_id"].as_str().unwrap()).unwrap();
    let other_host = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("other-host"),
            Some(credential_body(other, "host")),
        )
        .await
        .status(201)
        .body;
    let mint = format!("{}/sessions", f.workspace_path());
    f.client
        .call(
            "POST",
            &mint,
            Some(&f.host),
            None,
            Some("foreign-target"),
            Some(json!({"credential_id":other_host["credential_id"]})),
        )
        .await
        .error(404, "not_found");
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("foreign-workspace"),
            Some(credential_body(other, "worker")),
        )
        .await
        .error(404, "not_found");
    let mut null_workspace = credential_body(f.workspace, "worker");
    null_workspace["workspace_id"] = Value::Null;
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("null-workspace"),
            Some(null_workspace),
        )
        .await
        .error(400, "invalid_input");
    sqlx::query(
        "UPDATE credentials SET expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1",
    )
    .bind(f.host.id)
    .execute(&f.pool)
    .await
    .unwrap();
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("unbounded-lifetime"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .error(403, "forbidden");
    let mut bounded = credential_body(f.workspace, "worker");
    let expiry: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM credentials WHERE id=$1")
            .bind(f.host.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    bounded["expires_at"] = json!(anchorfs_service::control::timestamp(expiry));
    f.client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.host),
            None,
            Some("bounded-lifetime"),
            Some(bounded),
        )
        .await
        .status(201);
}
