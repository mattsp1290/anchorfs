use crate::key::ObjectKey;
use anchorfs_core::digest::Sha256Digest;
use async_trait::async_trait;
use bytes::Bytes;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutOutcome {
    pub bytes_written: u64,
    pub sha256: Sha256Digest,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub key: ObjectKey,
    pub length: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    pub objects: Vec<ObjectMeta>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackendError {
    #[error("backend unavailable")]
    Unavailable,
    #[error("backend request timed out")]
    Timeout,
    #[error("object not found")]
    NotFound,
    #[error("object integrity mismatch")]
    IntegrityMismatch,
    #[error("backend outcome uncertain")]
    UncertainOutcome,
    #[error("backend access forbidden")]
    Forbidden,
    #[error("invalid byte range")]
    InvalidRange,
    #[error("bucket not found")]
    BucketNotFound,
    #[error("object exceeds single-object bound")]
    ObjectTooLarge,
    #[error("other backend error")]
    Other,
}
/// Immutable objects bounded by `Limits::default().chunk_size`.
/// Repeating identical bytes at the same key is idempotent. Callers allocate
/// unique keys for different bytes. Automatic deletion is intentionally absent.
#[async_trait]
pub trait ObjectBackend: Send + Sync {
    async fn put(
        &self,
        key: &ObjectKey,
        bytes: Bytes,
        expected_sha256: Sha256Digest,
    ) -> Result<PutOutcome, BackendError>;
    /// Half-open byte interval, with an exclusive upper bound.
    async fn get_range(&self, key: &ObjectKey, range: Range<u64>) -> Result<Bytes, BackendError>;
    /// Whole-object digest and length verification; ranged reads verify framing only.
    async fn get_verified(
        &self,
        key: &ObjectKey,
        expected_sha256: Sha256Digest,
        expected_length: u64,
    ) -> Result<Bytes, BackendError>;
    /// Reachability and list permission under the deployment prefix, not write permission.
    async fn probe(&self) -> Result<(), BackendError>;
    async fn head(&self, key: &ObjectKey) -> Result<ObjectMeta, BackendError>;
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ListPage, BackendError>;
}
