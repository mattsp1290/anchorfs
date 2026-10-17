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
    #[error("backend operations are not implemented in stage 0")]
    NotImplemented,
    #[error("other backend error")]
    Other,
}
/// Immutable object access. Automatic deletion is intentionally unavailable.
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
    async fn head(&self, key: &ObjectKey) -> Result<ObjectMeta, BackendError>;
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<ListPage, BackendError>;
}
