use super::support::*;
use crate::common::proxy::{self, Fault};

#[tokio::test]
async fn by1_writes_append_and_sparse_truncate_publish_complete_manifests() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let other = open(&f, &f.host, &f.host_session, "f", false).await;
    let first = write(&f, &handle, "first", 0, b"hello")
        .await
        .status(200)
        .json();
    assert_eq!(first["offset"], 0);
    assert_eq!(first["bytes_written"], 5);
    f.invariants().await;
    let added = append(&f, &handle, "append", b"world")
        .await
        .status(200)
        .json();
    assert_eq!(added["offset"], 5);
    assert_eq!(committed(&f, &other).await, b"helloworld");
    f.invariants().await;
    write(&f, &handle, "partial", 2, b"XY").await.status(200);
    assert_eq!(committed(&f, &other).await, b"heXYoworld");
    f.invariants().await;
    write(&f, &handle, "boundary", 4094, b"ABCDEF")
        .await
        .status(200);
    let mut expected = b"heXYoworld".to_vec();
    expected.resize(4094, 0);
    expected.extend_from_slice(b"ABCDEF");
    assert_eq!(committed(&f, &other).await, expected);
    f.invariants().await;
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected");
    };
    let puts = proxy.count(proxy::is_put);
    let path = format!("{}/truncate", handle_path(&f, &handle));
    f.client
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("shrink"),
            Some(json!({"size":2})),
        )
        .await
        .status(200);
    f.invariants().await;
    let grown = f
        .client
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("grow"),
            Some(json!({"size":4100})),
        )
        .await
        .status(200)
        .body;
    let same = f
        .client
        .call(
            "POST",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("same"),
            Some(json!({"size":4100})),
        )
        .await
        .status(200)
        .body;
    assert_eq!(same["generation"], grown["generation"]);
    let mut expected = b"he".to_vec();
    expected.resize(4100, 0);
    assert_eq!(committed(&f, &other).await, expected);
    assert_eq!(proxy.count(proxy::is_put), puts, "truncate must not upload");
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let synced = f
        .client
        .call(
            "POST",
            &format!("{}/fsync", handle_path(&f, &handle)),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(synced["generation"], grown["generation"]);
    assert_eq!(synced["size"], 4100);
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
        .bind(f.workspace)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(before, after, "fsync is receipt-free");
    f.invariants().await;
}

#[tokio::test]
async fn by3_lost_write_response_replays_before_handle_and_quota_validation() {
    let f = Fixture::with_bytes(
        workspace_body(),
        configure,
        Arc::new(CrashOnce::at(FaultPoint::AfterPublish)),
        false,
    )
    .await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let unknown = error(
        append(&f, &handle, "lost", b"once").await,
        503,
        "outcome_unknown",
    );
    let path = format!(
        "{}/operations/{}",
        f.workspace_path(),
        unknown["operation_id"].as_str().unwrap()
    );
    let receipt = f
        .client
        .call("GET", &path, Some(&f.host), None, None, None)
        .await
        .status(200)
        .body;
    assert_eq!(receipt["state"], "committed");
    assert_eq!(receipt["result"]["offset"], 0);
    assert_eq!(committed(&f, &handle).await, b"once");
    f.client
        .call(
            "DELETE",
            &handle_path(&f, &handle),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("close"),
            None,
        )
        .await
        .status(200);
    let replay = append(&f, &handle, "lost", b"once")
        .await
        .status(200)
        .json();
    assert_eq!(replay, receipt["result"]);
    assert_eq!(committed(&f, &handle).await, b"once");
    error(
        append(&f, &handle, "lost", b"changed").await,
        409,
        "idempotency_mismatch",
    );
    f.invariants().await;
}

#[tokio::test]
async fn by2_payload_and_offset_bind_the_key_and_empty_writes_admit_nothing() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "bound", 0, b"data").await.status(200);
    error(
        write(&f, &handle, "bound", 1, b"data").await,
        409,
        "idempotency_mismatch",
    );
    error(
        write(&f, &handle, "bound", 0, b"next").await,
        409,
        "idempotency_mismatch",
    );
    error(
        append(&f, &handle, "bound", b"data").await,
        409,
        "idempotency_mismatch",
    );
    error(
        write(&f, &handle, "empty", 0, b"").await,
        400,
        "invalid_input",
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND idempotency_key='empty'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(committed(&f, &handle).await, b"data");
    f.invariants().await;
}

#[tokio::test]
async fn by5_upload_failure_retains_charges_and_a_terminal_failed_receipt() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "before", 0, b"safe").await.status(200);
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected");
    };
    proxy.script_for(proxy::is_put, [Fault::DropRequest, Fault::DropRequest]);
    let failed = error(
        write(&f, &handle, "failed", 0, b"next").await,
        503,
        "dependency_unavailable",
    );
    assert_eq!(committed(&f, &handle).await, b"safe");
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
    assert_eq!(receipt["result"]["code"], "dependency_unavailable");
    error(
        write(&f, &handle, "failed", 0, b"next").await,
        409,
        "operation_failed",
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved,(SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1) FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(counts, (8, 0, 1));
    f.invariants().await;
}

#[tokio::test]
async fn by8_concurrent_appends_swap_reservations_at_the_staging_limit() {
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 2);
    let mut body = workspace_body();
    body["staging_bytes_limit"] = json!(4);
    let f = Fixture::with_bytes(body, configure, faults.clone(), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let a = append_task(&f, &handle, "a", b"AA");
    let b = append_task(&f, &handle, "b", b"BB");
    faults.wait().await;
    let open_transactions:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='idle in transaction'").fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        open_transactions, 0,
        "no transaction remains open between upload and publication"
    );
    let pending = f
        .client
        .call(
            "GET",
            &format!("{}/operations?idempotency_key=a", f.workspace_path()),
            Some(&f.worker),
            None,
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(pending["state"], "pending");
    f.invariants().await;
    release.send(true).unwrap();
    let a = a.await.unwrap().status(200).json();
    let b = b.await.unwrap().status(200).json();
    let mut offsets = [a["offset"].as_u64().unwrap(), b["offset"].as_u64().unwrap()];
    offsets.sort();
    assert_eq!(offsets, [0, 2]);
    let bytes = committed(&f, &handle).await;
    assert!(bytes == b"AABB" || bytes == b"BBAA");
    assert_eq!(append(&f, &handle, "a", b"AA").await.status(200).json(), a);
    assert_eq!(append(&f, &handle, "b", b"BB").await.status(200).json(), b);
    let values:(i64,i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved,(SELECT count(*) FROM upload_reservations WHERE workspace_id=$1) FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
    assert_eq!(values, (8, 0, 3));
    f.invariants().await;
}

#[tokio::test]
async fn by9_publication_rechecks_credential_epoch_and_handle_lifetime() {
    for mode in ["credential", "epoch", "handle"] {
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), false).await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let task = append_task(&f, &handle, "late", b"data");
        faults.wait().await;
        let (status, code) = match mode {
            "credential" => {
                f.client
                    .call(
                        "DELETE",
                        &format!("/v1/credentials/{}", f.worker.id),
                        Some(&f.host),
                        None,
                        Some("revoke-worker"),
                        None,
                    )
                    .await
                    .status(200);
                (401, "unauthenticated")
            }
            "epoch" => {
                sqlx::query("UPDATE workspaces SET mutation_epoch=mutation_epoch+1 WHERE id=$1")
                    .bind(f.workspace)
                    .execute(&f.pool)
                    .await
                    .unwrap();
                (409, "fenced")
            }
            "handle" => {
                sqlx::query("UPDATE handles SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=$2").bind(f.workspace).bind(Uuid::parse_str(handle["handle_id"].as_str().unwrap()).unwrap()).execute(&f.pool).await.unwrap();
                (410, "stale_handle")
            }
            _ => unreachable!(),
        };
        release.send(true).unwrap();
        let failed = error(task.await.unwrap(), status, code);
        assert_eq!(committed(&f, &handle).await, b"");
        let view = f
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
        assert_eq!(view["state"], "failed");
        assert_eq!(view["result"]["code"], code);
        let values:(i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(values, (4, 0));
        f.invariants().await;
    }
}
