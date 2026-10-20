use super::support::*;
use crate::common::proxy;

#[tokio::test]
async fn by7_physical_staging_principal_and_logical_limits_reject_before_upload() {
    let mut body = workspace_body();
    body["physical_bytes_limit"] = json!(12288);
    body["staging_bytes_limit"] = json!(8192);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected")
    };
    for index in 0..3 {
        write(
            &f,
            &handle,
            &format!("overwrite-{index}"),
            0,
            vec![index as u8; 4096],
        )
        .await
        .status(200);
        f.invariants().await;
    }
    let puts = proxy.count(proxy::is_put);
    error(
        write(&f, &handle, "physical", 0, vec![4; 4096]).await,
        507,
        "capacity_exhausted",
    );
    assert_eq!(proxy.count(proxy::is_put), puts);
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key='physical'),(SELECT count(*) FROM upload_reservations WHERE workspace_id=$1)").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(counts, (0, 3));
    sqlx::query("UPDATE workspace_quotas SET physical_bytes_limit=100000,staging_bytes_limit=4096 WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
    error(
        write(&f, &handle, "staging", 2048, vec![5; 4096]).await,
        507,
        "quota_exceeded",
    );
    sqlx::query("UPDATE credentials SET max_reservation_bytes=100 WHERE id=$1")
        .bind(f.worker.id)
        .execute(&f.pool)
        .await
        .unwrap();
    error(
        write(&f, &handle, "principal", 4096, vec![6; 101]).await,
        507,
        "quota_exceeded",
    );
    sqlx::query("UPDATE credentials SET max_reservation_bytes=100000 WHERE id=$1")
        .bind(f.worker.id)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE workspace_quotas SET logical_bytes_limit=4096 WHERE workspace_id=$1")
        .bind(f.workspace)
        .execute(&f.pool)
        .await
        .unwrap();
    error(
        write(&f, &handle, "logical", 4096, b"x").await,
        507,
        "quota_exceeded",
    );
    let reply = f
        .client
        .call(
            "POST",
            &format!("{}/truncate", handle_path(&f, &handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("logical-truncate"),
            Some(json!({"size":4097})),
        )
        .await
        .status(507)
        .body;
    assert_eq!(reply["code"], "quota_exceeded");
    assert_eq!(proxy.count(proxy::is_put), puts);
    f.invariants().await;
}

#[tokio::test]
async fn by7_publication_checks_logical_quota_again_and_charges_uploaded_orphans() {
    let mut body = workspace_body();
    body["logical_bytes_limit"] = json!(4);
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
    let f = Fixture::with_bytes(body, configure, faults.clone(), false).await;
    let a = open(&f, &f.worker, &f.worker_session, "a", true).await;
    let b = open(&f, &f.worker, &f.worker_session, "b", true).await;
    let first = append_task(&f, &a, "first", b"AAAA");
    faults.wait().await;
    append(&f, &b, "second", b"BBBB").await.status(200);
    release.send(true).unwrap();
    error(first.await.unwrap(), 507, "quota_exceeded");
    assert_eq!(
        read(&f, &a, 0, 4096, None).await.status(200).bytes,
        b"".as_slice()
    );
    assert_eq!(
        read(&f, &b, 0, 4096, None).await.status(200).bytes,
        b"BBBB".as_slice()
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved,logical_bytes_used FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(counts, (8, 0, 4));
    f.invariants().await;
}
