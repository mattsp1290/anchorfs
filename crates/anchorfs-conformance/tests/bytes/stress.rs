//! BY20: concurrent histories are checked independently of the chunk planners.
//! See D-C4 for why overwritten/truncated markers are checked at publication.
use super::support::*;
use crate::common::{
    minio,
    proxy::{self, Fault},
};
use rand::{RngCore, SeedableRng};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Schedule {
    calls: AtomicUsize,
    parked: AtomicUsize,
    crashed: Mutex<Vec<OperationId>>,
}
impl FaultInjector for Schedule {
    fn at(&self, point: FaultPoint, operation: Option<OperationId>) -> FaultAction {
        if point != FaultPoint::BeforePublish {
            return FaultAction::Continue;
        }
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call.is_multiple_of(17) {
            self.crashed.lock().unwrap().push(operation.unwrap());
            return FaultAction::Crash;
        }
        if call.is_multiple_of(3) {
            self.parked.fetch_add(1, Ordering::SeqCst);
            let (release, wait) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                let _ = release.send(true);
            });
            return FaultAction::Wait(wait);
        }
        FaultAction::Continue
    }
}
#[derive(Clone, Debug)]
enum Change {
    Write {
        offset: Option<usize>,
        bytes: Vec<u8>,
    },
    Truncate(usize),
}
struct Attempt {
    key: String,
    actor: usize,
    inode: i64,
    change: Change,
    acknowledged: Option<Value>,
}
fn actor(f: &Fixture, index: usize) -> (&Credential, &Value) {
    if index == 0 {
        (&f.worker, &f.worker_session)
    } else {
        (&f.host, &f.host_session)
    }
}
async fn invoke(
    f: &Fixture,
    index: usize,
    handle: &Value,
    key: &str,
    change: &Change,
) -> (u16, Value) {
    let (actor, session) = actor(f, index);
    match change {
        Change::Write { offset, bytes } => {
            let (method, path) = match offset {
                Some(offset) => (
                    "PUT",
                    format!("{}/bytes?offset={offset}", handle_path(f, handle)),
                ),
                None => ("POST", format!("{}/append", handle_path(f, handle))),
            };
            let reply = f
                .client
                .binary(
                    method,
                    &path,
                    Some(actor),
                    Some(session),
                    Some(key),
                    &[],
                    Body::from(bytes.clone()),
                )
                .await;
            (reply.status.as_u16(), reply.json())
        }
        Change::Truncate(size) => {
            let reply = f
                .client
                .call(
                    "POST",
                    &format!("{}/truncate", handle_path(f, handle)),
                    Some(actor),
                    Some(session),
                    Some(key),
                    Some(json!({"size":size})),
                )
                .await;
            (reply.status.as_u16(), reply.body)
        }
    }
}
fn expected_response(status: u16, value: &Value) {
    if status == 200 {
        return;
    }
    assert!(
        [404, 409, 410, 503, 507].contains(&status),
        "unexpected status {status}: {value}"
    );
    assert!(
        [
            "not_found",
            "stale_handle",
            "generation_conflict",
            "quota_exceeded",
            "capacity_exhausted",
            "operation_failed",
            "outcome_unknown",
            "dependency_unavailable"
        ]
        .contains(&value["code"].as_str().unwrap()),
        "unexpected error: {value}"
    );
}
async fn worker(f: Arc<Fixture>, index: usize, until: std::time::Instant) -> Vec<Attempt> {
    let actor_index = index % 2;
    let (credential, session) = actor(&f, actor_index);
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x1c2000 + index as u64);
    let mut handles = BTreeMap::new();
    let mut attempts = vec![];
    let mut sequence = 0;
    while std::time::Instant::now() < until {
        let file = (rng.next_u32() % 3) as usize;
        let path = format!("file-{file}");
        if let std::collections::btree_map::Entry::Vacant(entry) = handles.entry(file) {
            entry.insert(open(&f, credential, session, &path, true).await);
        }
        let handle = handles.get(&file).unwrap();
        let key = format!("stress-{index}-{sequence}");
        sequence += 1;
        let choice = rng.next_u32() % 10;
        if choice < 6 {
            let marker = format!("task-{index}-write-{sequence:08};").into_bytes();
            let change = match choice {
                0 | 1 => Change::Write {
                    offset: Some((rng.next_u32() % 2048) as usize),
                    bytes: marker,
                },
                2 | 3 => Change::Write {
                    offset: None,
                    bytes: marker,
                },
                _ => Change::Truncate((rng.next_u32() % 2048) as usize),
            };
            let first = invoke(&f, actor_index, handle, &key, &change);
            let (status, value) = if (rng.next_u32() % 4) == 0 {
                let duplicate = async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    invoke(&f, actor_index, handle, &key, &change).await
                };
                let (first, second) = tokio::join!(first, duplicate);
                expected_response(first.0, &first.1);
                expected_response(second.0, &second.1);
                if first.0 == 200 && second.0 == 200 {
                    assert_eq!(first.1, second.1, "same-key publication changed");
                }
                if second.0 == 200 { second } else { first }
            } else {
                first.await
            };
            expected_response(status, &value);
            attempts.push(Attempt {
                key,
                actor: actor_index,
                inode: handle["attributes"]["inode_id"].as_i64().unwrap(),
                change,
                acknowledged: (status == 200).then_some(value),
            });
        } else if choice == 6 {
            f.client
                .call(
                    "DELETE",
                    &handle_path(&f, handle),
                    Some(credential),
                    Some(session),
                    Some(&key),
                    None,
                )
                .await
                .status(200);
            handles.remove(&file);
        } else if choice == 7 {
            let reply = f
                .client
                .call(
                    "DELETE",
                    &format!("{}/files?path={path}", f.workspace_path()),
                    Some(credential),
                    Some(session),
                    Some(&key),
                    None,
                )
                .await;
            assert!(
                [200, 404].contains(&reply.status.as_u16()),
                "unlink: {}",
                reply.body
            );
            // Keep the old handle: subsequent writes may target an unlinked inode.
            open(&f, credential, session, &path, true).await;
        } else if choice == 8 {
            if let Some(last) = attempts.last() {
                let reply = f
                    .client
                    .call(
                        "GET",
                        &format!(
                            "{}/operations?idempotency_key={}",
                            f.workspace_path(),
                            last.key
                        ),
                        Some(credential),
                        None,
                        None,
                        None,
                    )
                    .await;
                assert!(
                    [200, 404].contains(&reply.status.as_u16()),
                    "receipt: {}",
                    reply.body
                );
            }
        } else {
            read_as(&f, handle, credential, session, 0, 4096, None)
                .await
                .status(200);
        }
    }
    assert!(sequence > 10, "worker did not exercise enough operations");
    attempts
}

async fn manifest_bytes(
    f: &Fixture,
    operation: Uuid,
    cache: &mut BTreeMap<Uuid, Vec<u8>>,
) -> Vec<u8> {
    let (id, size): (Uuid, i64) = sqlx::query_as(
        "SELECT id,size FROM manifests WHERE workspace_id=$1 AND created_by_operation_id=$2",
    )
    .bind(f.workspace)
    .bind(operation)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let rows:Vec<(i32,Uuid,i32)>=sqlx::query_as("SELECT chunk_index,object_id,logical_length FROM manifest_chunks WHERE workspace_id=$1 AND manifest_id=$2").bind(f.workspace).bind(id).fetch_all(&f.pool).await.unwrap();
    let Backend::Minio {
        endpoint,
        deployment,
        ..
    } = &f.backend
    else {
        panic!("MinIO expected")
    };
    let raw = minio::raw_client(endpoint).await;
    let mut result = vec![0; size as usize];
    for (index, object, length) in rows {
        if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(object) {
            entry.insert(
                minio::raw_get(&raw, &format!("{deployment}/{}/{object}", f.workspace))
                    .await
                    .unwrap(),
            );
        }
        let start = index as usize * 4096;
        result[start..start + length as usize].copy_from_slice(&cache[&object][..length as usize]);
    }
    result
}

#[tokio::test]
async fn by20_randomized_concurrency_has_no_deadlocks_lost_publications_or_pending_receipts() {
    let mut body = workspace_body();
    body["logical_bytes_limit"] = json!(1 << 30);
    body["physical_bytes_limit"] = json!(1 << 30);
    body["staging_bytes_limit"] = json!(4096);
    body["logical_inodes_limit"] = json!(100000);
    let schedule = Arc::new(Schedule::default());
    let f = Arc::new(Fixture::with_bytes(body, configure, schedule.clone(), true).await);
    let mut initial = vec![];
    for index in 0..3 {
        initial.push(
            open(
                &f,
                &f.worker,
                &f.worker_session,
                &format!("file-{index}"),
                true,
            )
            .await,
        );
    }
    error(
        append(&f, &initial[0], "stress-crash", vec![0; 4096]).await,
        503,
        "outcome_unknown",
    );
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND state='active'").bind(f.workspace).execute(&f.pool).await.unwrap();
    let probe = append(&f, &initial[0], "stress-probe", b"pressure-probe;")
        .await
        .status(200)
        .json();
    let expired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM upload_reservations WHERE workspace_id=$1 AND state='expired'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(expired > 0, "quota-pressure fencing was not exercised");
    let mut attempts = vec![Attempt {
        key: "stress-probe".into(),
        actor: 0,
        inode: initial[0]["attributes"]["inode_id"].as_i64().unwrap(),
        change: Change::Write {
            offset: None,
            bytes: b"pressure-probe;".to_vec(),
        },
        acknowledged: Some(probe),
    }];
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected")
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x1c20);
    for matcher in [proxy::is_get_object as proxy::Matcher, proxy::is_put] {
        proxy.script_for(
            matcher,
            (0..20000).map(|_| Fault::Delay(Duration::from_millis((rng.next_u32() % 9) as u64))),
        );
    }
    let before: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let pool = f.pool.clone();
    let faults = schedule.clone();
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let expiry = tokio::spawn(async move {
        while !*stopped.borrow_and_update() {
            let crashed = std::mem::take(&mut *faults.crashed.lock().unwrap());
            for id in crashed {
                sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE operation_id=$1 AND state='active'").bind(id.0).execute(&pool).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let until = std::time::Instant::now() + Duration::from_secs(20);
    let mut tasks = vec![];
    for index in 0..8 {
        tasks.push(tokio::spawn(worker(f.clone(), index, until)));
    }
    for task in tasks {
        attempts.extend(task.await.unwrap());
    }
    stop.send(true).unwrap();
    expiry.await.unwrap();
    assert!(schedule.parked.load(Ordering::SeqCst) > 0);
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND state='active'").bind(f.workspace).execute(&f.pool).await.unwrap();
    let mut histories: BTreeMap<i64, Vec<(i64, Change, Value)>> = BTreeMap::new();
    let mut cache = BTreeMap::new();
    let mut acknowledged = 0;
    for attempt in attempts {
        let (actor, _) = actor(&f, attempt.actor);
        let reply = f
            .client
            .call(
                "GET",
                &format!(
                    "{}/operations?idempotency_key={}",
                    f.workspace_path(),
                    attempt.key
                ),
                Some(actor),
                None,
                None,
                None,
            )
            .await;
        if reply.status.as_u16() == 404 {
            assert!(attempt.acknowledged.is_none());
            continue;
        }
        let receipt = reply.status(200).body;
        assert_ne!(receipt["state"], "pending");
        if receipt["state"] != "committed" {
            assert!(attempt.acknowledged.is_none());
            continue;
        }
        let result = receipt["result"].clone();
        if let Some(ack) = attempt.acknowledged {
            acknowledged += 1;
            assert_eq!(ack, result);
            if let Change::Write { bytes, .. } = &attempt.change {
                let snapshot = manifest_bytes(
                    &f,
                    result["operation_id"].as_str().unwrap().parse().unwrap(),
                    &mut cache,
                )
                .await;
                let offset = result["offset"].as_u64().unwrap() as usize;
                assert_eq!(&snapshot[offset..offset + bytes.len()], bytes);
                assert_eq!(
                    snapshot
                        .windows(bytes.len())
                        .filter(|window| *window == bytes)
                        .count(),
                    1,
                    "acknowledged marker duplicated at publication"
                );
            }
        }
        histories.entry(attempt.inode).or_default().push((
            result["generation"].as_i64().unwrap(),
            attempt.change,
            result,
        ));
    }
    assert!(acknowledged > 20, "insufficient acknowledged history");
    let inodes: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM inodes WHERE workspace_id=$1 AND kind='file'")
            .bind(f.workspace)
            .fetch_all(&f.pool)
            .await
            .unwrap();
    let mut models = BTreeMap::new();
    for inode in inodes {
        let mut history = histories.remove(&inode).unwrap_or_default();
        history.sort_by_key(|entry| entry.0);
        let mut bytes = vec![];
        for (_, change, result) in history {
            match change {
                Change::Write {
                    offset,
                    bytes: payload,
                } => {
                    let actual = result["offset"].as_u64().unwrap() as usize;
                    assert_eq!(actual, offset.unwrap_or(bytes.len()));
                    bytes.resize(bytes.len().max(actual + payload.len()), 0);
                    bytes[actual..actual + payload.len()].copy_from_slice(&payload);
                }
                Change::Truncate(size) => bytes.resize(size, 0),
            }
        }
        assert_eq!(
            committed(&f, &json!({"attributes":{"inode_id":inode}})).await,
            bytes,
            "final inode {inode} differs from committed history"
        );
        models.insert(inode, bytes);
    }
    assert!(histories.is_empty());
    for index in 0..3 {
        let handle = open(
            &f,
            &f.worker,
            &f.worker_session,
            &format!("file-{index}"),
            false,
        )
        .await;
        let expected = &models[&handle["attributes"]["inode_id"].as_i64().unwrap()];
        for offset in (0..=expected.len()).step_by(4096) {
            assert_eq!(
                read(&f, &handle, offset as u64, 4096, None)
                    .await
                    .status(200)
                    .bytes,
                expected[offset..expected.len().min(offset + 4096)]
            );
        }
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND state='pending'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(pending, 0);
    f.invariants().await;
    let after: i64 = sqlx::query_scalar(
        "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        after, before,
        "concurrent workload caused a PostgreSQL deadlock"
    );
    eprintln!(
        "BY20: 8 tasks, 20 seconds, {acknowledged} acknowledged byte mutations, {} parked publications, deadlocks {before}->{after}",
        schedule.parked.load(Ordering::SeqCst)
    );
}
