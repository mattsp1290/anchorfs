//! Independent table oracle from spec 03: class selection and directory search.
use crate::common::fixture::*;
fn open(path: &str, read: bool, write: bool, create: bool) -> Value {
    json!({"path":path,"read":read,"write":write,"create":create,"exclusive":false,"truncate":false,"mode":384})
}
#[tokio::test]
async fn ns5_read_role_cannot_mutate_or_gain_writable_handles() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    f.client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("file"),
            Some(open("f", true, true, true)),
        )
        .await
        .status(201);
    let reader = f.credential("worker", "read", 1000, 1000).await;
    let session = f.session_for(&reader).await;
    for (key, body) in [
        ("create-read", open("new", true, false, true)),
        ("write", open("f", false, true, false)),
    ] {
        f.client
            .call(
                "POST",
                &format!("{prefix}/handles"),
                Some(&reader),
                Some(&session),
                Some(key),
                Some(body),
            )
            .await
            .error(403, "forbidden");
    }
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&reader),
            Some(&session),
            Some("read-mkdir"),
            Some(json!({"path":"d","mode":493})),
        )
        .await
        .error(403, "forbidden");
    let grant = f
        .client
        .call(
            "POST",
            &format!("{prefix}/handles"),
            Some(&reader),
            Some(&session),
            Some("read"),
            Some(open("f", true, false, false)),
        )
        .await
        .status(201)
        .body;
    let handle = format!("{prefix}/handles/{}", grant["handle_id"].as_str().unwrap());
    f.client
        .call(
            "POST",
            &format!("{handle}/renew"),
            Some(&reader),
            Some(&session),
            Some("renew"),
            None,
        )
        .await
        .status(200);
    f.client
        .call(
            "DELETE",
            &handle,
            Some(&reader),
            Some(&session),
            Some("close"),
            None,
        )
        .await
        .status(200);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key=ANY($2)",
    )
    .bind(f.workspace)
    .bind(vec!["create-read", "write", "read-mkdir"])
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    f.invariants().await;
}
#[tokio::test]
async fn ns5_owner_group_other_permission_matrix() {
    let mut workspace = workspace_body();
    workspace["root_mode"] = json!(511);
    workspace["logical_inodes_limit"] = json!(1000);
    let f = Fixture::with(workspace, |_| {}).await;
    let prefix = f.workspace_path();
    let admin = f.credential("host", "admin", 1000, 1000).await;
    let admin_session = f.session_for(&admin).await;
    let actors = [
        f.credential("worker", "write", 1000, 1000).await,
        f.credential("worker", "write", 2000, 1000).await,
        f.credential("worker", "write", 2000, 2000).await,
    ];
    for (class, actor) in actors.iter().enumerate() {
        let session = f.session_for(actor).await;
        for bits in 0..8_u32 {
            let dir = format!("c{class}b{bits}");
            f.client
                .call(
                    "POST",
                    &format!("{prefix}/directories"),
                    Some(&admin),
                    Some(&admin_session),
                    Some(&Uuid::new_v4().to_string()),
                    Some(json!({"path":dir,"mode":511})),
                )
                .await
                .status(200);
            for name in ["f", "remove", "move"] {
                f.client
                    .call(
                        "POST",
                        &format!("{prefix}/handles"),
                        Some(&admin),
                        Some(&admin_session),
                        Some(&Uuid::new_v4().to_string()),
                        Some(open(&format!("{dir}/{name}"), true, true, true)),
                    )
                    .await
                    .status(201);
            }
            let mode = bits << [6, 3, 0][class];
            for target in [&dir, &format!("{dir}/f")] {
                f.client
                    .call(
                        "PATCH",
                        &format!("{prefix}/attributes"),
                        Some(&admin),
                        Some(&admin_session),
                        Some(&Uuid::new_v4().to_string()),
                        Some(json!({"path":target,"mode":mode})),
                    )
                    .await
                    .status(200);
            }
            // Fixed independent truth table, including stat's lack of a target permission check.
            let x = bits & 1 != 0;
            let r = bits & 4 != 0;
            let w = bits & 2 != 0;
            for (method, suffix, body, allowed, success) in [
                ("GET", format!("/stat?path={dir}/f"), None, x, 200),
                ("GET", format!("/directory?path={dir}"), None, r && x, 200),
                (
                    "POST",
                    "/directories".into(),
                    Some(json!({"path":format!("{dir}/new"),"mode":493})),
                    w && x,
                    200,
                ),
                (
                    "DELETE",
                    format!("/files?path={dir}/remove"),
                    None,
                    w && x,
                    200,
                ),
                (
                    "POST",
                    "/rename".into(),
                    Some(json!({"from":format!("{dir}/move"),"to":format!("{dir}/moved")})),
                    w && x,
                    200,
                ),
                (
                    "POST",
                    "/handles".into(),
                    Some(open(&format!("{dir}/f"), true, false, false)),
                    r && x,
                    201,
                ),
                (
                    "POST",
                    "/handles".into(),
                    Some(open(&format!("{dir}/f"), false, true, false)),
                    w && x,
                    201,
                ),
                (
                    "PATCH",
                    "/attributes".into(),
                    Some(json!({"path":dir,"mode":mode})),
                    class == 0,
                    200,
                ),
                (
                    "PATCH",
                    "/attributes".into(),
                    Some(json!({"path":dir,"uid":3000})),
                    false,
                    200,
                ),
            ] {
                let reply = f
                    .client
                    .call(
                        method,
                        &format!("{prefix}{suffix}"),
                        Some(actor),
                        Some(&session),
                        Some(&Uuid::new_v4().to_string()),
                        body,
                    )
                    .await;
                if allowed {
                    reply.status(success);
                } else {
                    reply.error(403, "forbidden");
                }
            }
            f.invariants().await;
        }
    }
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&admin),
            Some(&admin_session),
            Some("private"),
            Some(json!({"path":"private","mode":0})),
        )
        .await
        .status(200);
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&admin),
            Some(&admin_session),
            Some("bypass"),
            Some(json!({"path":"private/child","mode":0})),
        )
        .await
        .status(200);
    f.client
        .call(
            "PATCH",
            &format!("{prefix}/attributes"),
            Some(&admin),
            Some(&admin_session),
            Some("chown"),
            Some(json!({"path":"private","uid":3000,"gid":4000})),
        )
        .await
        .status(200);
    f.client
        .call(
            "PATCH",
            &format!("{prefix}/attributes"),
            Some(&admin),
            Some(&admin_session),
            Some("special"),
            Some(json!({"path":"private","mode":2541})),
        )
        .await
        .error(400, "invalid_input");
    f.invariants().await;
}
