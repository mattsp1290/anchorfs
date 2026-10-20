use super::support::*;
use crate::common::proxy;
use bytes::Bytes;
use futures_core::Stream;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

struct ChannelStream(tokio::sync::mpsc::Receiver<Result<Bytes, io::Error>>);
impl Stream for ChannelStream {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
struct Unreadable;
impl Stream for Unreadable {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        panic!("receipt replay must not poll its replacement body")
    }
}
fn stream_task(
    f: &Fixture,
    generation: i64,
    key: &str,
    length: usize,
    digest: String,
    body: Body,
) -> tokio::task::JoinHandle<RawReply> {
    let client = Client {
        app: f.client.app.clone(),
    };
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!(
        "{}/files?path=f&expected_generation={generation}",
        f.workspace_path()
    );
    let key = key.to_owned();
    tokio::spawn(async move {
        client
            .binary(
                "PUT",
                &path,
                Some(&actor),
                Some(&session),
                Some(&key),
                &[
                    ("Content-Length", &length.to_string()),
                    ("X-Anchorfs-Content-SHA256", &digest),
                ],
                body,
            )
            .await
    })
}

#[tokio::test]
async fn by3d_lost_replace_responses_resolve_for_nonempty_and_empty_files() {
    for data in [b"durable".as_slice(), b"".as_slice()] {
        let f = Fixture::with_bytes(
            workspace_body(),
            configure,
            Arc::new(CrashOnce::at(FaultPoint::AfterPublish)),
            false,
        )
        .await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let unknown = error(
            replace(&f, "f", 1, "lost", data).await,
            503,
            "outcome_unknown",
        );
        let receipt = f
            .client
            .call(
                "GET",
                &format!(
                    "{}/operations/{}",
                    f.workspace_path(),
                    unknown["operation_id"].as_str().unwrap()
                ),
                Some(&f.host),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body;
        assert_eq!(receipt["state"], "committed");
        assert_eq!(read(&f, &handle, 0, 32, None).await.status(200).bytes, data);
        let replay = stream_task(
            &f,
            1,
            "lost",
            data.len(),
            anchorfs_core::sha256(data).to_string(),
            Body::from_stream(Unreadable),
        )
        .await
        .unwrap()
        .status(200)
        .json();
        assert_eq!(replay, receipt["result"]);
        f.invariants().await;
    }
}

#[tokio::test]
async fn by14_replace_shape_is_validated_before_authentication_or_admission() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let path = format!("{}/files?path=f&expected_generation=1", f.workspace_path());
    for headers in [
        vec![],
        vec![("Content-Length", "4")],
        vec![
            ("Content-Length", "4"),
            ("X-Anchorfs-Content-SHA256", "bad"),
        ],
    ] {
        let failure = f
            .client
            .binary(
                "PUT",
                &path,
                None,
                Some(&f.worker_session),
                Some("shape"),
                &headers,
                Body::from("data"),
            )
            .await
            .status(400)
            .json();
        assert_eq!(failure["code"], "invalid_input");
    }
    let failure = f
        .client
        .binary(
            "GET",
            &format!("{}/bytes?offset=0&length=0", handle_path(&f, &handle)),
            None,
            Some(&f.worker_session),
            None,
            &[],
            Body::empty(),
        )
        .await
        .status(400)
        .json();
    assert_eq!(failure["code"], "invalid_input");
    let failure = f
        .client
        .binary(
            "GET",
            &format!("{}/bytes?offset=0&length=4097", handle_path(&f, &handle)),
            None,
            Some(&f.worker_session),
            None,
            &[],
            Body::empty(),
        )
        .await
        .status(400)
        .json();
    assert_eq!(failure["code"], "invalid_input");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE workspace_id=$1 AND op_type='replace_file'",
    )
    .bind(f.workspace)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    f.invariants().await;
}

#[tokio::test]
async fn by14_replace_uploads_before_the_body_ends_and_replay_does_not_poll() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let old = write(&f, &handle, "old", 0, b"old")
        .await
        .status(200)
        .json();
    let old_pin = pin(&f, "f", "old-pin").await;
    let generation = old["generation"].as_i64().unwrap();
    let data: Vec<_> = (0..8197).map(|n| (n % 251) as u8).collect();
    let digest = anchorfs_core::sha256(&data).to_string();
    let (send, receive) = tokio::sync::mpsc::channel(1);
    let task = stream_task(
        &f,
        generation,
        "replace",
        data.len(),
        digest.clone(),
        Body::from_stream(ChannelStream(receive)),
    );
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected");
    };
    let before = proxy.forwarded(proxy::is_put);
    send.send(Ok(Bytes::copy_from_slice(&data[..4096])))
        .await
        .unwrap();
    assert!(
        proxy
            .wait_forwarded(proxy::is_put, before + 1, Duration::from_secs(20))
            .await,
        "first chunk must upload before the rest of the body arrives"
    );
    assert!(!task.is_finished());
    assert_eq!(
        read(&f, &handle, 0, 3, None).await.status(200).bytes,
        b"old".as_slice()
    );
    send.send(Ok(Bytes::copy_from_slice(&data[4096..])))
        .await
        .unwrap();
    drop(send);
    let replaced = task.await.unwrap().status(200).json();
    let mut actual = Vec::new();
    for offset in [0, 4096, 8192] {
        actual.extend_from_slice(
            &read(&f, &handle, offset, 4096, None)
                .await
                .status(200)
                .bytes,
        );
    }
    assert_eq!(actual, data);
    assert_eq!(committed(&f, &handle).await, data);
    let id = Uuid::parse_str(old_pin["read_pin_id"].as_str().unwrap()).unwrap();
    assert_eq!(
        read(&f, &handle, 0, 3, Some(id)).await.status(200).bytes,
        b"old".as_slice()
    );
    write(&f, &handle, "advance", 0, b"X").await.status(200);
    let replay = stream_task(
        &f,
        generation,
        "replace",
        data.len(),
        digest,
        Body::from_stream(Unreadable),
    )
    .await
    .unwrap()
    .status(200)
    .json();
    assert_eq!(replay, replaced);
    error(
        replace(&f, "f", generation, "replace", b"different").await,
        409,
        "idempotency_mismatch",
    );
    f.invariants().await;
}

#[tokio::test]
async fn by8_conditional_replace_rejects_data_and_metadata_changes_without_retry() {
    for metadata_only in [true, false] {
        let (faults, release) = GateFault::new(FaultPoint::BeforePublish, 1);
        let f = Fixture::with_bytes(workspace_body(), configure, faults.clone(), true).await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let task = stream_task(
            &f,
            1,
            "conditional",
            4,
            anchorfs_core::sha256(b"data").to_string(),
            Body::from("data"),
        );
        faults.wait().await;
        if metadata_only {
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
        } else {
            write(&f, &handle, "racer", 0, b"other").await.status(200);
        }
        release.send(true).unwrap();
        let failed = error(task.await.unwrap(), 409, "generation_conflict");
        assert_eq!(
            read(&f, &handle, 0, 4096, None).await.status(200).bytes,
            if metadata_only {
                b"".as_slice()
            } else {
                b"other".as_slice()
            }
        );
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
        assert_eq!(view["result"]["code"], "generation_conflict");
        let Backend::Minio {
            proxy: Some(proxy), ..
        } = &f.backend
        else {
            panic!("proxy expected");
        };
        assert_eq!(
            proxy.count(proxy::is_put),
            if metadata_only { 1 } else { 2 }
        );
        let candidates: i64 =
            sqlx::query_scalar("SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(candidates, 1);
        f.invariants().await;
    }
}

#[tokio::test]
async fn by14b_invalid_digest_and_lengths_fail_without_publication_and_empty_replace_is_atomic() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let generation = write(&f, &handle, "safe", 0, b"safe")
        .await
        .status(200)
        .json()["generation"]
        .as_i64()
        .unwrap();
    for (key, length, digest, body, message) in [
        (
            "digest",
            4,
            anchorfs_core::sha256(b"xxxx").to_string(),
            b"next".to_vec(),
            "content_digest_mismatch",
        ),
        (
            "short",
            8,
            anchorfs_core::sha256(b"short").to_string(),
            b"few".to_vec(),
            "content_length_mismatch",
        ),
        (
            "long",
            3,
            anchorfs_core::sha256(b"too").to_string(),
            b"tool".to_vec(),
            "content_length_mismatch",
        ),
    ] {
        let failed = error(
            stream_task(&f, generation, key, length, digest, Body::from(body))
                .await
                .unwrap(),
            400,
            "invalid_input",
        );
        assert_eq!(failed["message"], message);
        assert_eq!(
            read(&f, &handle, 0, 4, None).await.status(200).bytes,
            b"safe".as_slice()
        );
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
        assert_eq!(view["result"]["code"], "invalid_input");
        f.invariants().await;
    }
    let Backend::Minio {
        proxy: Some(proxy), ..
    } = &f.backend
    else {
        panic!("proxy expected");
    };
    let puts = proxy.count(proxy::is_put);
    let empty = replace(&f, "f", generation, "empty", b"")
        .await
        .status(200)
        .json();
    assert!(
        read(&f, &handle, 0, 1, None)
            .await
            .status(200)
            .bytes
            .is_empty()
    );
    assert_eq!(proxy.count(proxy::is_put), puts);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM upload_reservations WHERE workspace_id=$1 AND operation_id=$2",
    )
    .bind(f.workspace)
    .bind(Uuid::parse_str(empty["operation_id"].as_str().unwrap()).unwrap())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
    let failed = error(
        stream_task(
            &f,
            empty["generation"].as_i64().unwrap(),
            "bad-empty",
            0,
            "ab".repeat(32),
            Body::empty(),
        )
        .await
        .unwrap(),
        400,
        "invalid_input",
    );
    assert_eq!(failed["message"], "content_digest_mismatch");
    f.invariants().await;
}

#[tokio::test]
async fn by14b_stream_error_and_hard_deadline_finalize_the_owned_reservation() {
    for deadline in [false, true] {
        let (faults, release) = GateFault::new(FaultPoint::AfterReserve, 1);
        let f = Fixture::with_bytes(
            workspace_body(),
            |c| {
                configure(c);
                c.max_data_ops = 1;
                if deadline {
                    c.data_call_deadline = Duration::from_millis(1500);
                    c.db_lock_timeout = Duration::from_millis(100);
                }
            },
            faults.clone(),
            true,
        )
        .await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        let (send, receive) = tokio::sync::mpsc::channel(1);
        let data = vec![42u8; 8192];
        let task = stream_task(
            &f,
            1,
            "broken",
            data.len(),
            anchorfs_core::sha256(&data).to_string(),
            Body::from_stream(ChannelStream(receive)),
        );
        faults.wait().await;
        release.send(true).unwrap();
        {
            send.send(Ok(Bytes::copy_from_slice(&data[..4096])))
                .await
                .unwrap();
            let Backend::Minio {
                proxy: Some(proxy), ..
            } = &f.backend
            else {
                panic!("proxy expected");
            };
            assert!(
                proxy
                    .wait_forwarded(proxy::is_put, 1, Duration::from_secs(20))
                    .await
            );
            if !deadline {
                send.send(Err(io::Error::other("synthetic stream disconnect")))
                    .await
                    .unwrap();
            }
        }
        let failure = task.await.unwrap().status(503).json();
        assert_eq!(
            failure["code"],
            if deadline {
                "outcome_unknown"
            } else {
                "dependency_unavailable"
            }
        );
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let state:String=sqlx::query_scalar("SELECT state FROM operations WHERE workspace_id=$1 AND idempotency_key='broken'").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
                if state=="failed" {break;}
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        drop(send);
        assert!(
            read(&f, &handle, 0, 1, None)
                .await
                .status(200)
                .bytes
                .is_empty()
        );
        let counters:(i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(counters, (4096, 0));
        let reservation_state: String =
            sqlx::query_scalar("SELECT state FROM upload_reservations WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(reservation_state, "failed");
        write(&f, &handle, "following", 0, b"ok").await.status(200);
        f.invariants().await;
    }
}
