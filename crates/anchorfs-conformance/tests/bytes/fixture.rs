use crate::common::{fixture::*, minio};
use anchorfs_service::pipeline::faults::NoFaults;

#[tokio::test]
async fn byte_fixtures_share_minio_but_isolate_metadata_and_keys() {
    let configure = |c: &mut Config| {
        c.chunk_size = 4096;
        c.max_request_bytes = 4096;
    };
    let (a, b) = tokio::join!(
        Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true),
        Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), false),
    );
    assert_ne!(a.workspace, b.workspace);
    assert_ne!(a.config.deployment_id, b.config.deployment_id);
    let (
        Backend::Minio {
            endpoint: a_endpoint,
            proxy: a_proxy,
            ..
        },
        Backend::Minio {
            endpoint: b_endpoint,
            ..
        },
    ) = (&a.backend, &b.backend)
    else {
        panic!("MinIO fixtures expected");
    };
    assert_eq!(a_endpoint, b_endpoint);
    assert!(a_proxy.is_some());
    assert_eq!(a.config.chunk_size, 4096);
    let stored: i32 = sqlx::query_scalar("SELECT chunk_size FROM schema_meta")
        .fetch_one(&a.pool)
        .await
        .unwrap();
    assert_eq!(stored, 4096);
    let bound: i64 =
        sqlx::query_scalar("SELECT max_reservation_bytes FROM credentials WHERE id=$1")
            .bind(a.worker.id)
            .fetch_one(&a.pool)
            .await
            .unwrap();
    assert_eq!(bound, 1 << 30);
    a.invariants().await;
    b.invariants().await;
}

#[tokio::test]
async fn bucket_oracle_rejects_an_uncharged_object() {
    let f = Fixture::with_bytes(workspace_body(), |_| {}, Arc::new(NoFaults), false).await;
    let Backend::Minio {
        endpoint,
        deployment,
        ..
    } = &f.backend
    else {
        panic!("MinIO expected");
    };
    let client = minio::raw_client(endpoint).await;
    let key = format!("{deployment}/{}/{}", f.workspace, Uuid::new_v4());
    minio::raw_put(&client, &key, b"uncharged synthetic bytes".to_vec()).await;
    let task = tokio::spawn(async move {
        f.invariants().await;
    });
    let error = task
        .await
        .expect_err("oracle must reject an uncharged bucket object");
    assert!(error.is_panic());
    let payload = error.into_panic();
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        message.contains("bucket-uncharged:"),
        "unexpected oracle failure: {message}"
    );
}
