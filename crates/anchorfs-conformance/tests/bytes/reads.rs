use super::support::*;
use crate::common::minio;

#[tokio::test]
async fn by1_two_chunk_sparse_write_is_read_identically_by_another_session() {
    use rand::{RngCore, SeedableRng};
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let mut payload = vec![0; 4096];
    rand::rngs::StdRng::seed_from_u64(0x1c01).fill_bytes(&mut payload);
    let written = write(&f, &handle, "sparse", 2048, payload.clone())
        .await
        .status(200)
        .json();
    let mut expected = vec![0; 2048];
    expected.extend_from_slice(&payload);
    f.invariants().await;
    let other = open(&f, &f.host, &f.host_session, "f", false).await;
    let mut actual = Vec::new();
    for offset in [0, 4096] {
        let read = read_as(&f, &other, &f.host, &f.host_session, offset, 4096, None)
            .await
            .status(200);
        assert_eq!(
            read.headers["X-Anchorfs-Generation"].to_str().unwrap(),
            written["generation"].as_i64().unwrap().to_string()
        );
        actual.extend_from_slice(&read.bytes);
    }
    assert_eq!(actual, expected);
    let stat = f
        .client
        .call(
            "GET",
            &format!("{}/stat?path=f", f.workspace_path()),
            Some(&f.host),
            Some(&f.host_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(stat["size"], 6144);
    assert_eq!(stat["generation"], written["generation"]);
    let equal:bool=sqlx::query_scalar("SELECT m.generation=i.generation FROM inodes i JOIN manifests m ON (m.workspace_id,m.id)=(i.workspace_id,i.current_manifest_id) WHERE i.workspace_id=$1 AND i.id=$2")
        .bind(f.workspace).bind(handle["attributes"]["inode_id"].as_i64().unwrap()).fetch_one(&f.pool).await.unwrap();
    assert!(equal);
    f.invariants().await;
}

#[tokio::test]
async fn by1_read_uses_one_manifest_while_replace_publishes_a_new_generation() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "a", 0, vec![b'A'; 4096])
        .await
        .status(200);
    let old = append(&f, &handle, "b", vec![b'B'; 4096])
        .await
        .status(200)
        .json();
    let (faults, release) = GateFault::new(FaultPoint::AfterPeek, 1);
    let mut state = AppState::production((*f.config).clone(), f.pool.clone(), f.backend.service());
    state.faults = faults.clone();
    let client = Client {
        app: build_router(state),
    };
    let actor = f.worker.clone();
    let session = f.worker_session.clone();
    let path = format!("{}/bytes?offset=4094&length=4", handle_path(&f, &handle));
    let reader = tokio::spawn(async move {
        client
            .binary(
                "GET",
                &path,
                Some(&actor),
                Some(&session),
                None,
                &[],
                Body::empty(),
            )
            .await
    });
    faults.wait().await;
    let transactions:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND state='idle in transaction'").fetch_one(&f.pool).await.unwrap();
    assert_eq!(transactions, 0);
    replace(
        &f,
        "f",
        old["generation"].as_i64().unwrap(),
        "new",
        vec![b'C'; 8192],
    )
    .await
    .status(200);
    release.send(true).unwrap();
    let old_read = reader.await.unwrap().status(200);
    assert_eq!(old_read.bytes, b"AABB".as_slice());
    assert_eq!(
        old_read.headers["X-Anchorfs-Generation"].to_str().unwrap(),
        old["generation"].as_i64().unwrap().to_string()
    );
    assert_eq!(
        read(&f, &handle, 4094, 4, None).await.status(200).bytes,
        b"CCCC".as_slice()
    );
    f.invariants().await;
}

#[tokio::test]
async fn by12_reads_verify_whole_objects_including_discarded_tails() {
    let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false).await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    write(&f, &handle, "data", 0, b"abSECRET").await.status(200);
    for size in [2, 8] {
        f.client
            .call(
                "POST",
                &format!("{}/truncate", handle_path(&f, &handle)),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(&format!("size-{size}")),
                Some(json!({"size":size})),
            )
            .await
            .status(200);
    }
    let bytes = read(&f, &handle, 0, 8, None).await.status(200);
    assert_eq!(bytes.bytes, b"ab\0\0\0\0\0\0".as_slice());
    assert_eq!(bytes.headers["Content-Type"], "application/octet-stream");
    assert!(
        read(&f, &handle, 8, 1, None)
            .await
            .status(200)
            .bytes
            .is_empty()
    );
    let object: Uuid =
        sqlx::query_scalar("SELECT id FROM objects WHERE workspace_id=$1 AND state='published'")
            .bind(f.workspace)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let unaffected = open(&f, &f.worker, &f.worker_session, "unaffected", true).await;
    write(&f, &unaffected, "unaffected", 0, b"healthy")
        .await
        .status(200);
    let Backend::Minio {
        endpoint,
        deployment,
        ..
    } = &f.backend
    else {
        panic!("MinIO expected");
    };
    let client = minio::raw_client(endpoint).await;
    let key = format!("{deployment}/{}/{object}", f.workspace);
    minio::raw_put(&client, &key, b"abxxxxxx".to_vec()).await;
    let failed = read(&f, &handle, 0, 1, None).await.status(503).json();
    assert_eq!(failed["message"], "backend_integrity");
    assert!(failed["retry_after_ms"].is_null());
    assert_eq!(
        read(&f, &unaffected, 0, 4096, None).await.status(200).bytes,
        b"healthy".as_slice()
    );
    client
        .delete_object()
        .bucket(minio::BUCKET)
        .key(&key)
        .send()
        .await
        .unwrap();
    let missing = read(&f, &handle, 0, 1, None).await.status(503).json();
    assert_eq!(missing["message"], "backend_integrity");
    assert_eq!(
        read(&f, &unaffected, 0, 4096, None).await.status(200).bytes,
        b"healthy".as_slice()
    );
    minio::raw_put(&client, &key, b"abSECRET".to_vec()).await;
    assert_eq!(
        read(&f, &handle, 0, 8, None).await.status(200).bytes,
        b"ab\0\0\0\0\0\0".as_slice()
    );
    f.invariants().await;
}

#[tokio::test]
async fn by13_pins_fix_data_generation_and_are_session_bound_bounded_and_expiring() {
    let f = Fixture::with_bytes(
        workspace_body(),
        |c| {
            configure(c);
            c.max_read_pins_per_session = 2;
        },
        Arc::new(NoFaults),
        false,
    )
    .await;
    let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
    let one = write(&f, &handle, "one", 0, b"one")
        .await
        .status(200)
        .json();
    let first = pin(&f, "f", "pin-one").await;
    let id = Uuid::parse_str(first["read_pin_id"].as_str().unwrap()).unwrap();
    let two = write(&f, &handle, "two", 0, b"two")
        .await
        .status(200)
        .json();
    assert_eq!(
        read(&f, &handle, 0, 3, None).await.status(200).bytes,
        b"two".as_slice()
    );
    let old = read(&f, &handle, 0, 3, Some(id)).await.status(200);
    assert_eq!(old.bytes, b"one".as_slice());
    assert_eq!(
        old.headers["X-Anchorfs-Generation"].to_str().unwrap(),
        one["generation"].as_i64().unwrap().to_string()
    );
    let changed = f
        .client
        .call(
            "PATCH",
            &format!("{}/attributes", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("chmod"),
            Some(json!({"path":"f","mode":384})),
        )
        .await
        .status(200)
        .body;
    let second = pin(&f, "f", "pin-two").await;
    assert_eq!(second["generation"], two["generation"]);
    assert_eq!(
        read(&f, &handle, 0, 3, None).await.status(200).headers["X-Anchorfs-Generation"]
            .to_str()
            .unwrap(),
        changed["generation"].as_i64().unwrap().to_string()
    );
    let other = open(&f, &f.worker, &f.worker_session, "other", true).await;
    let mismatch = error(read(&f, &other, 0, 1, Some(id)).await, 400, "invalid_input");
    assert_eq!(mismatch["message"], "pin_mismatch");
    let session = f.session_for(&f.worker).await;
    let foreign = open(&f, &f.worker, &session, "f", false).await;
    error(
        read_as(&f, &foreign, &f.worker, &session, 0, 3, Some(id)).await,
        404,
        "not_found",
    );
    f.client
        .call(
            "POST",
            &format!("{}/read-pins", f.workspace_path()),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("too-many"),
            Some(json!({"path":"f"})),
        )
        .await
        .error(507, "quota_exceeded");
    let path = format!("{}/read-pins/{id}", f.workspace_path());
    let released = f
        .client
        .call(
            "DELETE",
            &path,
            Some(&f.worker),
            Some(&f.worker_session),
            Some("release"),
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(
        f.client
            .call(
                "DELETE",
                &path,
                Some(&f.worker),
                Some(&f.worker_session),
                Some("release"),
                None
            )
            .await
            .status(200)
            .body,
        released
    );
    error(read(&f, &handle, 0, 3, Some(id)).await, 410, "stale_handle");
    sqlx::query("UPDATE read_pins SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(f.workspace).execute(&f.pool).await.unwrap();
    let second_id = Uuid::parse_str(second["read_pin_id"].as_str().unwrap()).unwrap();
    error(
        read(&f, &handle, 0, 3, Some(second_id)).await,
        410,
        "stale_handle",
    );
    // Reader-side pins remain available while an exclusive workflow is active.
    sqlx::query("UPDATE workspaces SET mutation_epoch=mutation_epoch+1,workflow_owner_credential_id=$2,workflow_token_hash=$3,workflow_expires_at=clock_timestamp()+interval '1 hour' WHERE id=$1")
        .bind(f.workspace).bind(f.host.id).bind(anchorfs_core::sha256([0u8;32]).0.as_slice()).execute(&f.pool).await.unwrap();
    pin(&f, "f", "during-lease").await;
    assert_eq!(
        read(&f, &handle, 0, 3, None).await.status(200).bytes,
        b"two".as_slice()
    );
    error(
        write(&f, &handle, "busy", 0, b"no").await,
        409,
        "workspace_busy",
    );
    f.invariants().await;
}
