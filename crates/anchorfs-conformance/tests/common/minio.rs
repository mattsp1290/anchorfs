#![allow(dead_code)]
use anchorfs_backend::config::{Addressing, ChecksumMode, CredentialSource, S3Config};
use anchorfs_core::{DeploymentId, Secret, digest::Sha256Digest};
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region, retry::RetryConfig},
    primitives::ByteStream,
};
use reqwest::Url;
use std::time::{Duration, Instant};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, core::IntoContainerPort, runners::AsyncRunner,
};

pub const MINIO_IMAGE: &str = "bitnamilegacy/minio";
pub const MINIO_TAG: &str = "2025.7.23-debian-12-r5";
pub const MINIO_IMAGE_DIGEST: &str =
    "sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20";
pub const BUCKET: &str = "anchorfs-test";
pub const ROOT_USER: &str = "anchorfs-fixture";
pub const ROOT_PASSWORD: &str = "synthetic-minio-fixture-password";

pub struct Minio {
    pub container: ContainerAsync<GenericImage>,
    pub endpoint: Url,
}

pub async fn start() -> Minio {
    let container = GenericImage::new(MINIO_IMAGE, &format!("{MINIO_TAG}@{MINIO_IMAGE_DIGEST}"))
        .with_exposed_port(9000.tcp())
        .with_env_var("MINIO_ROOT_USER", ROOT_USER)
        .with_env_var("MINIO_ROOT_PASSWORD", ROOT_PASSWORD)
        .with_startup_timeout(Duration::from_secs(60))
        .start()
        .await
        .expect("start disposable MinIO");
    let endpoint = format!(
        "http://127.0.0.1:{}",
        container.get_host_port_ipv4(9000).await.unwrap()
    )
    .parse()
    .unwrap();
    let client = raw_client(&endpoint).await;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if matches!(
            tokio::time::timeout(Duration::from_secs(5), client.list_buckets().send()).await,
            Ok(Ok(_))
        ) {
            break;
        }
        if Instant::now() >= deadline {
            let stdout = container.stdout_to_vec().await.unwrap_or_default();
            let stderr = container.stderr_to_vec().await.unwrap_or_default();
            let logs = format!(
                "{}\n{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
            let lines: Vec<_> = logs.lines().collect();
            panic!(
                "MinIO failed readiness:\n{}",
                lines[lines.len().saturating_sub(50)..].join("\n")
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if let Err(error) = client.create_bucket().bucket(BUCKET).send().await {
        let accepted = error
            .as_service_error()
            .is_some_and(|e| e.is_bucket_already_owned_by_you() || e.is_bucket_already_exists());
        assert!(accepted, "create fixture bucket failed: {error}");
    }
    for _ in 0..2 {
        client
            .head_bucket()
            .bucket(BUCKET)
            .send()
            .await
            .expect("fixture bucket ready");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Minio {
        container,
        endpoint,
    }
}

pub fn deployment() -> DeploymentId {
    DeploymentId::parse(format!(
        "t-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ))
    .unwrap()
}

pub fn backend_config(
    endpoint: &Url,
    deployment: DeploymentId,
    mode: ChecksumMode,
    request_timeout: Duration,
    max_retries: u32,
) -> S3Config {
    S3Config {
        endpoint: endpoint.clone(),
        region: "us-east-1".into(),
        addressing: Addressing::Path,
        bucket: BUCKET.into(),
        deployment_id: deployment,
        ca_bundle_path: None,
        credential_source: CredentialSource::Static {
            access_key_id: ROOT_USER.into(),
            secret_access_key: Secret::new(ROOT_PASSWORD.into()),
        },
        request_timeout,
        max_retries,
        allow_plaintext: true,
        checksum_mode: mode,
    }
}

pub async fn raw_client(endpoint: &Url) -> Client {
    Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint.as_str())
            .force_path_style(true)
            .credentials_provider(Credentials::new(
                ROOT_USER,
                ROOT_PASSWORD,
                None,
                None,
                "synthetic-fixture",
            ))
            .retry_config(RetryConfig::disabled())
            .build(),
    )
}

pub async fn raw_get(client: &Client, key: &str) -> Option<Vec<u8>> {
    match client.get_object().bucket(BUCKET).key(key).send().await {
        Ok(output) => Some(output.body.collect().await.unwrap().into_bytes().to_vec()),
        Err(error) if error.as_service_error().is_some_and(|e| e.is_no_such_key()) => None,
        Err(error) => panic!("independent GET failed: {error}"),
    }
}

pub async fn raw_put(client: &Client, key: &str, bytes: impl Into<Vec<u8>>) {
    client
        .put_object()
        .bucket(BUCKET)
        .key(key)
        .body(ByteStream::from(bytes.into()))
        .send()
        .await
        .expect("plant synthetic object");
}

pub async fn raw_count(client: &Client, prefix: &str) -> usize {
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = client
            .list_objects_v2()
            .bucket(BUCKET)
            .prefix(prefix)
            .set_continuation_token(cursor)
            .send()
            .await
            .unwrap();
        count += page.contents().len();
        if !page.is_truncated().unwrap_or(false) {
            return count;
        }
        cursor = Some(
            page.next_continuation_token()
                .expect("truncated page has cursor")
                .to_owned(),
        );
    }
}

pub fn base64_digest(d: &Sha256Digest) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in d.0.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        encoded.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}
