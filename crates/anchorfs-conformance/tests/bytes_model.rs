//! Sequential byte semantics against an independently maintained dense oracle.
//! Concurrent publication schedules are exercised separately by the BY tests.
mod common;
#[path = "bytes/support.rs"]
#[allow(dead_code)]
mod support;
use proptest::{
    prelude::*,
    test_runner::{Config as PropertyConfig, TestRunner},
};
use std::collections::BTreeMap;
use support::*;

#[derive(Debug, Clone)]
struct Op {
    file: usize,
    kind: u8,
    offset: usize,
    bytes: Vec<u8>,
    stale: bool,
}
fn operations() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(
        (
            0..3usize,
            0..10u8,
            0..=12288usize,
            prop::collection::vec(any::<u8>(), 1..=4096),
            any::<bool>(),
        )
            .prop_map(|(file, kind, offset, bytes, stale)| Op {
                file,
                kind,
                offset,
                bytes,
                stale,
            }),
        1..=12,
    )
}
struct File {
    bytes: Vec<u8>,
    generation: i64,
    data_generation: i64,
    handle: Value,
}

#[test]
fn byte_model_sequences() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _entered = runtime.enter();
    let mut body = workspace_body();
    for column in [
        "logical_bytes_limit",
        "physical_bytes_limit",
        "staging_bytes_limit",
    ] {
        body[column] = json!(1_i64 << 40);
    }
    body["logical_inodes_limit"] = json!(1_000_000);
    let f = runtime.block_on(Fixture::with_bytes(
        body,
        configure,
        Arc::new(NoFaults),
        false,
    ));
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let mut runner = TestRunner::new(PropertyConfig {
        cases,
        max_shrink_iters: 128,
        failure_persistence: None,
        ..Default::default()
    });
    let started = std::time::Instant::now();
    runner
        .run(&operations(), |ops| {
            runtime.block_on(run_case(&f, ops));
            Ok(())
        })
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(300),
        "byte model exceeded five-minute budget"
    );
    eprintln!("byte model: {cases} sequences in {:?}", started.elapsed());
}

async fn run_case(f: &Fixture, ops: Vec<Op>) {
    let prefix = Uuid::new_v4().to_string();
    let mut model = BTreeMap::new();
    for index in 0..3 {
        let path = format!("{prefix}-{index}");
        let handle = open(f, &f.worker, &f.worker_session, &path, true).await;
        let generation = handle["attributes"]["generation"].as_i64().unwrap();
        model.insert(
            path,
            File {
                bytes: vec![],
                generation,
                data_generation: generation,
                handle,
            },
        );
    }
    for op in ops {
        let path = format!("{prefix}-{}", op.file);
        let file = model.get_mut(&path).unwrap();
        let key = Uuid::new_v4().to_string();
        let mut changed_data = false;
        let result = match op.kind {
            0 | 1 => {
                let offset = if op.kind == 1 {
                    file.bytes.len()
                } else {
                    op.offset
                };
                let reply = if op.kind == 1 {
                    append(f, &file.handle, &key, op.bytes.as_slice()).await
                } else {
                    write(f, &file.handle, &key, offset as u64, op.bytes.as_slice()).await
                };
                let value = reply.status(200).json();
                assert_eq!(value["offset"], offset);
                file.bytes
                    .resize(file.bytes.len().max(offset + op.bytes.len()), 0);
                file.bytes[offset..offset + op.bytes.len()].copy_from_slice(&op.bytes);
                changed_data = true;
                Some(value)
            }
            2 => {
                let value = f
                    .client
                    .call(
                        "POST",
                        &format!("{}/truncate", handle_path(f, &file.handle)),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some(&key),
                        Some(json!({"size":op.offset})),
                    )
                    .await
                    .status(200)
                    .body;
                changed_data = file.bytes.len() != op.offset;
                file.bytes.resize(op.offset, 0);
                Some(value)
            }
            3 => {
                let bytes: Vec<_> = op.bytes.iter().copied().cycle().take(op.offset).collect();
                let reply = replace(
                    f,
                    &path,
                    file.generation - i64::from(op.stale),
                    &key,
                    bytes.as_slice(),
                )
                .await;
                if op.stale {
                    error(reply, 409, "generation_conflict");
                    None
                } else {
                    file.bytes = bytes;
                    changed_data = true;
                    Some(reply.status(200).json())
                }
            }
            4 => {
                let value = f
                    .client
                    .call(
                        "PATCH",
                        &format!("{}/attributes", f.workspace_path()),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some(&key),
                        Some(json!({"path":path,"mode":420})),
                    )
                    .await
                    .status(200)
                    .body;
                Some(value)
            }
            5 => {
                let pin = pin(f, &path, &key).await;
                assert_eq!(pin["generation"], file.data_generation);
                let id = pin["read_pin_id"].as_str().unwrap().parse().unwrap();
                let snapshot = file.bytes.clone();
                let appended = append(f, &file.handle, &Uuid::new_v4().to_string(), b"new")
                    .await
                    .status(200)
                    .json();
                file.bytes.extend_from_slice(b"new");
                changed_data = true;
                let reply = read(f, &file.handle, op.offset as u64, 4096, Some(id))
                    .await
                    .status(200);
                let start = op.offset.min(snapshot.len());
                assert_eq!(
                    reply.bytes,
                    snapshot[start..snapshot.len().min(start + 4096)]
                );
                assert_eq!(
                    reply.headers["X-Anchorfs-Generation"].to_str().unwrap(),
                    file.data_generation.to_string()
                );
                f.client
                    .call(
                        "DELETE",
                        &format!("{}/read-pins/{id}", f.workspace_path()),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some(&Uuid::new_v4().to_string()),
                        None,
                    )
                    .await
                    .status(200);
                Some(appended)
            }
            6 => {
                f.client
                    .call(
                        "DELETE",
                        &handle_path(f, &file.handle),
                        Some(&f.worker),
                        Some(&f.worker_session),
                        Some(&key),
                        None,
                    )
                    .await
                    .status(200);
                file.handle = open(f, &f.worker, &f.worker_session, &path, false).await;
                None
            }
            7 => {
                let handle=f.client.call("POST",&format!("{}/handles",f.workspace_path()),Some(&f.worker),Some(&f.worker_session),Some(&key),Some(json!({"path":path,"read":true,"write":true,"create":false,"exclusive":false,"truncate":true,"mode":420}))).await.status(201).body;
                changed_data = true;
                file.bytes.clear();
                let attrs = handle["attributes"].clone();
                file.handle = handle;
                Some(attrs)
            }
            8 => {
                file.handle = open(f, &f.worker, &f.worker_session, &path, true).await;
                None
            }
            _ => {
                let reply = read(
                    f,
                    &file.handle,
                    op.offset as u64,
                    op.bytes.len() as u32,
                    None,
                )
                .await
                .status(200);
                let start = op.offset.min(file.bytes.len());
                assert_eq!(
                    reply.bytes,
                    file.bytes[start..file.bytes.len().min(start + op.bytes.len())]
                );
                None
            }
        };
        if let Some(result) = result {
            let observed = result["generation"].as_i64().unwrap();
            if changed_data {
                assert_eq!(observed, file.generation + 1);
                file.data_generation = observed;
            } else if op.kind == 4 {
                assert_eq!(observed, file.generation + 1);
            } else {
                assert_eq!(observed, file.generation);
            }
            file.generation = observed;
        }
        for (path, file) in &model {
            let stat = f
                .client
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
                .body;
            assert_eq!(stat["size"], file.bytes.len());
            assert_eq!(stat["generation"], file.generation);
            for offset in (0..=file.bytes.len()).step_by(4096) {
                let reply = read(f, &file.handle, offset as u64, 4096, None)
                    .await
                    .status(200);
                assert_eq!(
                    reply.bytes,
                    file.bytes[offset..file.bytes.len().min(offset + 4096)]
                );
                assert_eq!(
                    reply.headers["X-Anchorfs-Generation"].to_str().unwrap(),
                    file.generation.to_string()
                );
            }
        }
        f.invariants().await;
    }
}
