use super::support::*;
use crate::common::proxy::{self, Fault};

fn client_with(f: &Fixture, faults: Arc<dyn FaultInjector>) -> Client {
    let mut state = AppState::production((*f.config).clone(), f.pool.clone(), f.backend.service());
    state.faults = faults;
    Client {
        app: build_router(state),
    }
}
fn put_task(
    f: &Fixture,
    client: Client,
    handle: &Value,
    key: &str,
    offset: u64,
    bytes: &[u8],
) -> tokio::task::JoinHandle<RawReply> {
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!("{}/bytes?offset={offset}", handle_path(f, handle));
    let key = key.to_string();
    let bytes = bytes.to_vec();
    tokio::spawn(async move {
        client
            .binary(
                "PUT",
                &path,
                Some(&actor),
                Some(&session),
                Some(&key),
                &[],
                Body::from(bytes),
            )
            .await
    })
}
async fn no_idle_transaction(f: &Fixture) {
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='idle in transaction'").fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        count, 0,
        "byte pipeline retained a transaction across an external wait"
    );
}

#[tokio::test]
async fn by1c_overlapping_writers_preserve_unrelated_bytes_after_retry() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let a = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let b = open(&f, &f.host, &f.host_session, "f", false).await;
    write(&f, &a, "seed", 0, b"abcdefghij").await.status(200);
    let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
    let first = put_task(&f, client_with(&f, faults.clone()), &a, "first", 2, b"AAAA");
    faults.wait().await;
    let second = f
        .client
        .binary(
            "PUT",
            &format!("{}/bytes?offset=4", handle_path(&f, &b)),
            Some(&f.host),
            Some(&f.host_session),
            Some("second"),
            &[],
            Body::from("BBBB"),
        )
        .await
        .status(200)
        .json();
    assert_eq!(
        read(&f, &a, 0, 4096, None).await.status(200).bytes,
        b"abcdBBBBij".as_slice()
    );
    no_idle_transaction(&f).await;
    release.send(true).unwrap();
    let first = first.await.unwrap().status(200).json();
    assert_eq!(
        first["generation"].as_i64().unwrap(),
        second["generation"].as_i64().unwrap() + 1
    );
    assert_eq!(
        read(&f, &a, 0, 4096, None).await.status(200).bytes,
        b"abAAAABBij".as_slice()
    );
    assert_eq!(
        write(&f, &a, "first", 2, b"AAAA").await.status(200).json(),
        first
    );
    f.invariants().await;
}

#[tokio::test]
async fn by17_pipeline_fault_waits_and_delayed_backend_io_hold_no_transaction() {
    let mut body = workspace_body();
    body["physical_bytes_limit"] = json!(100_000);
    let f = Fixture::with_bytes(body, configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "seed", 0, b"seed").await.status(200);
    for point in [
        FaultPoint::AfterPeek,
        FaultPoint::BeforeGet,
        FaultPoint::BeforeUpload,
        FaultPoint::AfterUpload,
        FaultPoint::BeforePublish,
    ] {
        let (faults, release) = GateFault::new(point, 1);
        let task = put_task(
            &f,
            client_with(&f, faults.clone()),
            &handle,
            &format!("{point:?}"),
            1,
            b"XY",
        );
        faults.wait().await;
        no_idle_transaction(&f).await;
        release.send(true).unwrap();
        task.await.unwrap().status(200);
        f.invariants().await;
    }
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected")
    };
    for (index, matcher) in [proxy::is_get_object as proxy::Matcher, proxy::is_put]
        .into_iter()
        .enumerate()
    {
        let before = proxy.count(matcher);
        proxy.script_for(matcher, [Fault::Delay(Duration::from_millis(500))]);
        let task = put_task(
            &f,
            client_with(&f, Arc::new(NoFaults)),
            &handle,
            &format!("delayed-{index}"),
            0,
            b"Z",
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while proxy.count(matcher) == before {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        no_idle_transaction(&f).await;
        task.await.unwrap().status(200);
        f.invariants().await;
    }
}

struct LoseEveryResponse;
impl FaultInjector for LoseEveryResponse {
    fn at(&self, point: FaultPoint, _: Option<OperationId>) -> FaultAction {
        if point == FaultPoint::AfterPublish {
            FaultAction::Crash
        } else {
            FaultAction::Continue
        }
    }
}
#[tokio::test]
async fn by19_two_lost_append_responses_resolve_to_distinct_offsets() {
    let f = Fixture::with_bytes(
        workspace_body(),
        configure,
        Arc::new(LoseEveryResponse),
        false,
    )
    .await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let a = append_task(&f, &handle, "a", b"AAAA");
    let b = append_task(&f, &handle, "b", b"BBBB");
    let a = error(a.await.unwrap(), 503, "outcome_unknown");
    let b = error(b.await.unwrap(), 503, "outcome_unknown");
    assert_ne!(a["operation_id"], b["operation_id"]);
    let a = append(&f, &handle, "a", b"AAAA").await.status(200).json();
    let b = append(&f, &handle, "b", b"BBBB").await.status(200).json();
    let mut offsets = [a["offset"].as_u64().unwrap(), b["offset"].as_u64().unwrap()];
    offsets.sort();
    assert_eq!(offsets, [0, 4]);
    let bytes = read(&f, &handle, 0, 4096, None).await.status(200).bytes;
    assert!(bytes == b"AAAABBBB".as_slice() || bytes == b"BBBBAAAA".as_slice());
    f.invariants().await;
}

#[tokio::test]
async fn by8_generation_retry_is_bounded_and_ignores_metadata_only_changes() {
    for mode in ["exhausted", "metadata", "truncate"] {
        let f = Fixture::with_bytes(
            workspace_body(),
            |c| {
                configure(c);
                if mode == "exhausted" {
                    c.max_retry_attempts = 1;
                }
            },
            Arc::new(NoFaults),
            false,
        )
        .await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        write(&f, &handle, "seed", 0, b"abcdefghij")
            .await
            .status(200);
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let pending = put_task(
            &f,
            client_with(&f, faults.clone()),
            &handle,
            "pending",
            8,
            b"XY",
        );
        faults.wait().await;
        match mode {
            "exhausted" => {
                write(&f, &handle, "racer", 0, b"! ").await.status(200);
            }
            "metadata" => {
                f.client
                    .call(
                        "PATCH",
                        &format!("{}/attributes", f.workspace_path()),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some("chmod"),
                        Some(json!({"path":"f","mode":384})),
                    )
                    .await
                    .status(200);
            }
            "truncate" => {
                f.client
                    .call(
                        "POST",
                        &format!("{}/truncate", handle_path(&f, &handle)),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some("truncate"),
                        Some(json!({"size":2})),
                    )
                    .await
                    .status(200);
            }
            _ => unreachable!(),
        }
        release.send(true).unwrap();
        let response = pending.await.unwrap();
        if mode == "exhausted" {
            error(response, 409, "generation_conflict");
            error(
                write(&f, &handle, "pending", 8, b"XY").await,
                409,
                "operation_failed",
            );
        } else {
            response.status(200);
        }
        let expected: &[u8] = match mode {
            "exhausted" => b"! cdefghij",
            "metadata" => b"abcdefghXY",
            "truncate" => b"ab\0\0\0\0\0\0XY",
            _ => unreachable!(),
        };
        assert_eq!(
            read(&f, &handle, 0, 4096, None).await.status(200).bytes,
            expected
        );
        let attempts:i64=sqlx::query_scalar("SELECT count(*) FROM upload_reservations r JOIN operations p ON p.id=r.operation_id WHERE p.workspace_id=$1 AND p.idempotency_key='pending'").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(attempts, if mode == "truncate" { 2 } else { 1 });
        f.invariants().await;
    }
}

#[tokio::test]
async fn by19a_different_files_publish_concurrently() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let a = open(&f, &f.worker, &f.worker_session, "a", true).await;
    let b = open(&f, &f.host, &f.host_session, "b", true).await;
    let actor = f.host.clone();
    let session = f.host_session.clone();
    let client = Client {
        app: f.client.app.clone(),
    };
    let path = format!("{}/bytes?offset=0", handle_path(&f, &b));
    let started = std::time::Instant::now();
    let first = put_task(
        &f,
        Client {
            app: f.client.app.clone(),
        },
        &a,
        "a",
        0,
        &vec![1; 4096],
    );
    let second = tokio::spawn(async move {
        client
            .binary(
                "PUT",
                &path,
                Some(&actor),
                Some(&session),
                Some("b"),
                &[],
                Body::from(vec![2; 4096]),
            )
            .await
    });
    first.await.unwrap().status(200);
    second.await.unwrap().status(200);
    eprintln!(
        "BY19a two sessions / different files: {:?} wall time (observation, no SLO)",
        started.elapsed()
    );
    assert_eq!(
        read(&f, &a, 0, 4096, None).await.status(200).bytes,
        vec![1; 4096]
    );
    assert_eq!(
        read_as(&f, &b, &f.host, &f.host_session, 0, 4096, None)
            .await
            .status(200)
            .bytes,
        vec![2; 4096]
    );
    f.invariants().await;
}
