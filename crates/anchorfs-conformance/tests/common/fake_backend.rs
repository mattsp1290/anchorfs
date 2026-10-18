use anchorfs_backend::{key::ObjectKey, traits::*};
use anchorfs_core::Sha256Digest;
use bytes::Bytes;
use std::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub struct FakeBackend(AtomicUsize);
impl FakeBackend {
    pub fn untouched(&self) {
        assert_eq!(self.0.load(Ordering::SeqCst), 0);
    }
    fn called(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl ObjectBackend for FakeBackend {
    async fn put(
        &self,
        _: &ObjectKey,
        _: Bytes,
        _: Sha256Digest,
    ) -> Result<PutOutcome, BackendError> {
        self.called();
        Err(BackendError::Other)
    }
    async fn get_range(&self, _: &ObjectKey, _: Range<u64>) -> Result<Bytes, BackendError> {
        self.called();
        Err(BackendError::Other)
    }
    async fn get_verified(
        &self,
        _: &ObjectKey,
        _: Sha256Digest,
        _: u64,
    ) -> Result<Bytes, BackendError> {
        self.called();
        Err(BackendError::Other)
    }
    async fn probe(&self) -> Result<(), BackendError> {
        self.called();
        Ok(())
    }
    async fn head(&self, _: &ObjectKey) -> Result<ObjectMeta, BackendError> {
        self.called();
        Err(BackendError::Other)
    }
    async fn list(&self, _: &str, _: Option<&str>, _: u32) -> Result<ListPage, BackendError> {
        self.called();
        Err(BackendError::Other)
    }
}
