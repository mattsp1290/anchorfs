#[path = "common/minio.rs"]
mod minio;
#[path = "common/proxy.rs"]
mod proxy;
use anchorfs_backend::{
    config::{ChecksumMode, CredentialSource},
    key::ObjectKey,
    s3::S3Backend,
    traits::{BackendError, ObjectBackend},
};
use anchorfs_core::{DeploymentId, ObjectId, Secret, WorkspaceId, digest::sha256};
use bytes::Bytes;
use minio::*;
use proxy::*;
use rand::{RngCore, SeedableRng};
use std::time::{Duration, Instant};

fn data(len: usize) -> Bytes {
    let mut bytes = vec![0; len];
    rand::rngs::StdRng::seed_from_u64(0xa4c4_0f51).fill_bytes(&mut bytes);
    Bytes::from(bytes)
}
fn key(deployment: &DeploymentId) -> ObjectKey {
    ObjectKey::new(deployment, WorkspaceId::new(), ObjectId::new())
}
fn config(
    proxy: &Proxy,
    deployment: DeploymentId,
    mode: ChecksumMode,
) -> anchorfs_backend::config::S3Config {
    backend_config(
        &proxy.endpoint,
        deployment,
        mode,
        Duration::from_secs(20),
        2,
    )
}
fn backend(proxy: &Proxy, deployment: DeploymentId, mode: ChecksumMode) -> S3Backend {
    S3Backend::from_config(&config(proxy, deployment, mode)).unwrap()
}
const MODES: [ChecksumMode; 2] = [ChecksumMode::Sha256, ChecksumMode::None];

#[tokio::test]
async fn minio_fixture_boots() {
    for index in 0..3 {
        let minio = start().await;
        raw_client(&minio.endpoint)
            .await
            .head_bucket()
            .bucket(BUCKET)
            .send()
            .await
            .unwrap();
        if index == 2 {
            let mut output = minio
                .container
                .exec(testcontainers::core::ExecCommand::new([
                    "minio",
                    "--version",
                ]))
                .await
                .unwrap();
            let stdout = output.stdout_to_vec().await.unwrap();
            println!("MinIO version: {}", String::from_utf8_lossy(&stdout));
            assert_eq!(output.exit_code().await.unwrap(), Some(0));
        }
    }
}

#[tokio::test]
async fn proxy_is_transparent() {
    let minio = start().await;
    let proxy = Proxy::start(&minio.endpoint).await;
    let deployment = deployment();
    let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
    let key = key(&deployment);
    let bytes = data(1024 * 1024);
    backend
        .put(&key, bytes.clone(), sha256(&bytes))
        .await
        .unwrap();
    assert_eq!(
        backend
            .get_verified(&key, sha256(&bytes), bytes.len() as u64)
            .await
            .unwrap(),
        bytes
    );
    assert_eq!(
        raw_get(&raw_client(&minio.endpoint).await, key.as_str())
            .await
            .unwrap(),
        bytes
    );
    assert_eq!(proxy.forwarded(is_put), 1);
}

#[tokio::test]
async fn roundtrip_pagination_and_integrity() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), mode);
        let bytes = data(10_000);
        let digest = sha256(&bytes);
        let mut keys = Vec::new();
        for _ in 0..5 {
            let key = key(&deployment);
            let outcome = backend.put(&key, bytes.clone(), digest).await.unwrap();
            assert_eq!(outcome.bytes_written, bytes.len() as u64);
            assert_eq!(outcome.sha256, digest);
            assert_eq!(proxy.count(is_get_object), 0);
            keys.push(key);
        }
        let first_key = keys[0].clone();
        let key = &first_key;
        assert_eq!(backend.head(key).await.unwrap().length, bytes.len() as u64);
        assert_eq!(
            backend.get_range(key, 1024..4096).await.unwrap(),
            bytes.slice(1024..4096)
        );
        assert_eq!(
            backend
                .get_verified(key, digest, bytes.len() as u64)
                .await
                .unwrap(),
            bytes
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        let prefix = ObjectKey::deployment_prefix(&deployment);
        let mut cursor = None;
        let mut listed = Vec::new();
        let mut pages = 0;
        loop {
            let page = backend.list(&prefix, cursor.as_deref(), 2).await.unwrap();
            pages += 1;
            listed.extend(
                page.objects
                    .into_iter()
                    .map(|object| object.key.as_str().to_owned()),
            );
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        listed.sort();
        assert_eq!(
            listed,
            keys.iter()
                .map(|key| key.as_str().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(pages, 3);
        backend.list(&prefix, None, 0).await.unwrap();
        backend.list(&prefix, None, 5000).await.unwrap();
        assert_eq!(raw_count(&raw, &prefix).await, 5);
        assert_eq!(proxy.forwarded(is_put), 5);
        // F3: an independent observer overwrites bytes without changing their length.
        let deployment = minio::deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = self::backend(&proxy, deployment.clone(), mode);
        let fresh_key = self::key(&deployment);
        let key = &fresh_key;
        backend.put(key, bytes.clone(), digest).await.unwrap();
        let mut corrupt = bytes.to_vec();
        corrupt[3] ^= 0xff;
        raw_put(&raw, key.as_str(), corrupt.clone()).await;
        assert_eq!(
            backend.get_verified(key, digest, bytes.len() as u64).await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(backend.head(key).await.unwrap().length, bytes.len() as u64);
        assert_eq!(
            backend.get_range(key, 0..8).await.unwrap().as_ref(),
            &corrupt[..8]
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), corrupt);
    }
}

#[tokio::test]
async fn local_rejection_missing_credentials_and_bucket() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(100);
    // F2: mismatched caller digest is rejected without any PUT.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(b"different")).await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(proxy.count(is_put), 0);
        assert_eq!(raw_get(&raw, key.as_str()).await, None);
    }
    // F4: fresh objects return NotFound through all read forms.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        assert_eq!(backend.head(&key).await, Err(BackendError::NotFound));
        assert_eq!(
            backend.get_range(&key, 0..1).await,
            Err(BackendError::NotFound)
        );
        assert_eq!(
            backend
                .get_verified(&key, sha256(&bytes), bytes.len() as u64)
                .await,
            Err(BackendError::NotFound)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await, None);
    }
    // F5: a definitive credential failure is never repeated.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let mut config = config(&proxy, deployment.clone(), ChecksumMode::Sha256);
        config.credential_source = CredentialSource::Static {
            access_key_id: ROOT_USER.into(),
            secret_access_key: Secret::new("synthetic-wrong-secret".into()),
        };
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(&bytes)).await,
            Err(BackendError::Forbidden)
        );
        assert_eq!(backend.head(&key).await, Err(BackendError::Forbidden));
        assert_eq!(backend.probe().await, Err(BackendError::Forbidden));
        assert_eq!(raw_get(&raw, key.as_str()).await, None);
        assert_eq!(proxy.forwarded(is_put), 1);
    }
    // F13: bucket failures must not affect the real bucket.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let mut config = config(&proxy, deployment.clone(), ChecksumMode::Sha256);
        config.bucket = "absent-bucket".into();
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        let prefix = ObjectKey::deployment_prefix(&deployment);
        let before = raw_count(&raw, &prefix).await;
        assert_eq!(backend.probe().await, Err(BackendError::BucketNotFound));
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(&bytes)).await,
            Err(BackendError::BucketNotFound)
        );
        assert_eq!(raw_count(&raw, &prefix).await, before);
        assert_eq!(proxy.count(is_put), 1);
    }
}

#[tokio::test]
async fn throttling_and_gateway_reconciliation() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(1000);
    for (failures, expected_puts, expected_heads, stored) in
        [(2, 3, 0, true), (3, 4, 1, true), (6, 6, 2, false)]
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        proxy.script_for(
            is_put,
            std::iter::repeat_n(
                Fault::Status {
                    status: 503,
                    code: "SlowDown",
                },
                failures,
            ),
        );
        let result = backend.put(&key, bytes.clone(), sha256(&bytes)).await;
        if stored {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(result, Err(BackendError::Unavailable));
        }
        assert_eq!(
            raw_get(&raw, key.as_str()).await,
            stored.then(|| bytes.to_vec())
        );
        assert_eq!(proxy.count(is_put), expected_puts);
        assert_eq!(proxy.forwarded(is_put), usize::from(stored));
        assert_eq!(proxy.count(is_head_object), expected_heads);
    }
    // F6d: the object is stored despite a gateway response.
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        proxy.script_for(is_put, [Fault::ForwardThenStatus { status: 502 }]);
        let mut config = config(&proxy, deployment.clone(), mode);
        config.max_retries = 0;
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        assert_eq!(
            proxy.count(is_get_object),
            usize::from(mode == ChecksumMode::None)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        assert_eq!(proxy.forwarded(is_put), 1);
        assert_eq!(proxy.count(is_head_object), 1);
    }
}

#[tokio::test]
async fn lost_puts_and_late_delivery() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(1000);
    // F7a: a lost reply is reconciled without repeating the upload.
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        proxy.script_for(is_put, [Fault::SwallowResponse, Fault::Pass]);
        let config = backend_config(
            &proxy.endpoint,
            deployment.clone(),
            mode,
            Duration::from_secs(2),
            0,
        );
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        assert_eq!(
            proxy.count(is_get_object),
            usize::from(mode == ChecksumMode::None)
        );
        assert_eq!(proxy.forwarded(is_put), 1);
        assert_eq!(proxy.count(is_put), 1);
        assert_eq!(proxy.count(is_head_object), 1);
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
    }
    for fault in [
        Fault::DropRequest,
        Fault::DelayForward(Duration::from_secs(3)),
    ] {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let late = matches!(fault, Fault::DelayForward(_));
        proxy.script_for(is_put, [fault, Fault::Pass]);
        let config = backend_config(
            &proxy.endpoint,
            deployment.clone(),
            ChecksumMode::Sha256,
            Duration::from_secs(2),
            0,
        );
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        let start = Instant::now();
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        if late {
            assert!(start.elapsed() < Duration::from_secs(3));
            assert!(
                proxy
                    .wait_forwarded(is_put, 2, Duration::from_secs(10))
                    .await
            );
        }
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        assert_eq!(proxy.count(is_put), 2);
        assert_eq!(proxy.forwarded(is_put), if late { 2 } else { 1 });
        assert_eq!(proxy.count(is_head_object), 1);
    }
}

#[tokio::test]
async fn opaque_upload_outcomes() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(1000);
    for after_repeat in [false, true] {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        if after_repeat {
            proxy.script_for(is_put, [Fault::DropRequest, Fault::DropRequest]);
            proxy.script_for(is_head_object, [Fault::Pass, Fault::DropRequest]);
        } else {
            proxy.script_for(is_put, [Fault::SwallowResponse]);
            proxy.script_for(is_head_object, [Fault::DropRequest]);
        }
        let config = backend_config(
            &proxy.endpoint,
            deployment.clone(),
            ChecksumMode::Sha256,
            Duration::from_secs(2),
            0,
        );
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        let start = Instant::now();
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(&bytes)).await,
            Err(BackendError::UncertainOutcome)
        );
        assert_eq!(proxy.count(is_put), if after_repeat { 2 } else { 1 });
        assert_eq!(
            proxy.count(is_head_object),
            if after_repeat { 2 } else { 1 }
        );
        assert_eq!(proxy.count(is_get_object), 0);
        if after_repeat {
            assert_eq!(raw_get(&raw, key.as_str()).await, None);
        } else {
            assert!(start.elapsed() <= Duration::from_secs(5));
            assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn short_aborted_and_delayed_reads() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(10_000);
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), mode);
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        proxy.script_for(
            is_get_object,
            std::iter::repeat_n(
                Fault::TruncateBody {
                    keep: bytes.len() / 2,
                },
                2,
            ),
        );
        assert_eq!(
            backend
                .get_verified(&key, sha256(&bytes), bytes.len() as u64)
                .await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(
            backend.get_range(&key, 0..bytes.len() as u64).await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        assert_eq!(proxy.count(is_get_object), 2);
    }
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        proxy.script_for(
            is_get_object,
            std::iter::repeat_n(
                Fault::AbortMidBody {
                    after: bytes.len() / 2,
                },
                2,
            ),
        );
        assert_eq!(
            backend
                .get_verified(&key, sha256(&bytes), bytes.len() as u64)
                .await,
            Err(BackendError::Unavailable)
        );
        assert_eq!(
            backend.get_range(&key, 0..bytes.len() as u64).await,
            Err(BackendError::Unavailable)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
    }
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let config = backend_config(
            &proxy.endpoint,
            deployment.clone(),
            ChecksumMode::Sha256,
            Duration::from_secs(1),
            0,
        );
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        proxy.script_for(is_get_object, [Fault::Delay(Duration::from_secs(3))]);
        let start = Instant::now();
        assert_eq!(
            backend
                .get_verified(&key, sha256(&bytes), bytes.len() as u64)
                .await,
            Err(BackendError::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}

#[tokio::test]
async fn checksum_request_provider_and_payload_contracts() {
    use aws_sdk_s3::{
        error::ProvideErrorMetadata, primitives::ByteStream, types::ChecksumAlgorithm,
    };
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(10_000);
    // F11a: request headers reflect the selected checksum mode.
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), mode);
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        {
            let recorded = proxy.recorded.lock().unwrap();
            let request = recorded
                .iter()
                .find(|r| r.method == axum::http::Method::PUT)
                .unwrap();
            if mode == ChecksumMode::Sha256 {
                assert_eq!(
                    request.request_headers["x-amz-checksum-sha256"],
                    base64_digest(&sha256(&bytes))
                );
                assert_eq!(
                    request.request_headers["x-amz-sdk-checksum-algorithm"],
                    "SHA256"
                );
            } else {
                assert!(
                    !request
                        .request_headers
                        .keys()
                        .any(|header| header.as_str().starts_with("x-amz-checksum-"))
                );
                assert!(
                    !request
                        .request_headers
                        .contains_key("x-amz-sdk-checksum-algorithm")
                );
            }
        }
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
    }
    // F11b: use the direct SDK client, because a proxy cannot alter signed checksum headers.
    {
        let deployment = deployment();
        let key = key(&deployment);
        let error = raw
            .put_object()
            .bucket(BUCKET)
            .key(key.as_str())
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .checksum_sha256(base64_digest(&sha256(b"wrong digest")))
            .body(ByteStream::from(bytes.clone()))
            .send()
            .await
            .unwrap_err();
        assert!(error.raw_response().unwrap().status().is_client_error());
        let code = error.as_service_error().unwrap().code().unwrap();
        println!("F11b provider checksum code: {code}");
        assert!(["BadDigest", "XAmzContentChecksumMismatch", "InvalidRequest"].contains(&code));
        assert_eq!(raw_get(&raw, key.as_str()).await, None);
    }
    // F11c: body changes invalidate the signed payload hash in either checksum mode.
    for mode in MODES {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        proxy.script_for(is_put, [Fault::FlipRequestBodyByte { index: 10 }]);
        let mut config = config(&proxy, deployment.clone(), mode);
        config.max_retries = 0;
        let backend = S3Backend::from_config(&config).unwrap();
        let key = key(&deployment);
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(&bytes)).await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await, None);
        assert_eq!(proxy.forwarded(is_put), 1);
    }
}

#[tokio::test]
async fn absent_and_wrong_checksum_echoes() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    let bytes = data(1000);
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        proxy.script_for(
            is_put,
            [Fault::StripResponseHeader {
                name: "x-amz-checksum-sha256",
            }],
        );
        proxy.script_for(
            is_head_object,
            [Fault::StripResponseHeader {
                name: "x-amz-checksum-sha256",
            }],
        );
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        assert_eq!(proxy.count(is_get_object), 1);
        assert_eq!(proxy.count(is_head_object), 0);
        assert_eq!(proxy.forwarded(is_put), 1);
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
    }
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        // Standard Base64 encoding of an all-zero SHA-256 value, unrelated to these bytes.
        proxy.script_for(
            is_put,
            [Fault::MutateResponseHeader {
                name: "x-amz-checksum-sha256",
                value: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            }],
        );
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        assert_eq!(
            backend.put(&key, bytes.clone(), sha256(&bytes)).await,
            Err(BackendError::IntegrityMismatch)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
        assert_eq!(proxy.forwarded(is_put), 1);
    }
}

macro_rules! assert_variant {
    ($result:expr, $a:ident | $b:ident) => {
        assert!(matches!($result, Err(BackendError::$a | BackendError::$b)));
    };
}
#[tokio::test]
async fn endpoint_prefix_and_range_edges() {
    let minio = start().await;
    let raw = raw_client(&minio.endpoint).await;
    // F12: reserve then close a loopback port instead of assuming a system port is unused.
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        drop(listener);
        let deployment = deployment();
        let key = key(&deployment);
        let backend = S3Backend::from_config(&backend_config(
            &endpoint,
            deployment,
            ChecksumMode::Sha256,
            Duration::from_secs(1),
            0,
        ))
        .unwrap();
        let start = Instant::now();
        assert_variant!(backend.probe().await, Unavailable | Timeout);
        assert!(start.elapsed() < Duration::from_secs(3));
        let start = Instant::now();
        assert_variant!(backend.head(&key).await, Unavailable | Timeout);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    // F14: local prefix/key validation prevents requests, malformed stored keys are rejected.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        assert_eq!(
            backend.list("other/", None, 2).await,
            Err(BackendError::Other)
        );
        assert_eq!(proxy.count(is_list), 0);
        let foreign = key(&minio::deployment());
        assert_eq!(backend.head(&foreign).await, Err(BackendError::Other));
        assert_eq!(proxy.count(is_head_object), 0);
        let prefix = ObjectKey::deployment_prefix(&deployment);
        raw_put(&raw, &format!("{prefix}not-a-uuid/x"), b"1".to_vec()).await;
        assert_eq!(
            backend.list(&prefix, None, 2).await,
            Err(BackendError::Other)
        );
        assert_eq!(raw_count(&raw, &prefix).await, 1);
    }
    // F15: out-of-object and clamped ranges must not silently return partial data.
    {
        let deployment = deployment();
        let proxy = Proxy::start(&minio.endpoint).await;
        let backend = backend(&proxy, deployment.clone(), ChecksumMode::Sha256);
        let key = key(&deployment);
        let bytes = data(10_000);
        backend
            .put(&key, bytes.clone(), sha256(&bytes))
            .await
            .unwrap();
        assert_eq!(
            backend.get_range(&key, 10_000..10_001).await,
            Err(BackendError::InvalidRange)
        );
        assert_eq!(
            backend.get_range(&key, 9_000..11_000).await,
            Err(BackendError::InvalidRange)
        );
        let observed = proxy
            .recorded
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|record| is_get_object(&record.method, &record.path, &record.query))
            .unwrap()
            .upstream_status
            .unwrap();
        println!("F15 clamped range observed status: {observed}");
        assert_eq!(
            backend.get_range(&key, 9_000..10_000).await.unwrap(),
            bytes.slice(9000..)
        );
        assert_eq!(raw_get(&raw, key.as_str()).await.unwrap(), bytes);
    }
}
