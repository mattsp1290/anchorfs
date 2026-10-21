use crate::errno::{CallClass, ErrnoName, errno_for};
use anchorfs_client::{error::ClientError, retry::IdempotencyKey, wire::WriteResult};
use bytes::Bytes;
use std::{collections::BTreeMap, future::Future, time::Instant};
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct OpenFile {
    pub handle_id: Uuid,
    pub server_ino: i64,
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub fenced: bool,
    pub renewed: Instant,
}
#[derive(Default)]
pub struct OpenFiles {
    next: u64,
    pub entries: BTreeMap<u64, OpenFile>,
}
impl OpenFiles {
    pub fn insert(&mut self, file: OpenFile) -> u64 {
        self.next += 1;
        self.entries.insert(self.next, file);
        self.next
    }
}
#[derive(Debug, Clone)]
pub struct DirEntrySnapshot {
    pub ino: u64,
    pub name: String,
    pub kind: anchorfs_client::wire::InodeKind,
}
#[derive(Default)]
pub struct OpenDirs {
    next: u64,
    pub entries: BTreeMap<u64, Vec<DirEntrySnapshot>>,
}
impl OpenDirs {
    pub fn insert(&mut self, entries: Vec<DirEntrySnapshot>) -> u64 {
        self.next += 1;
        self.entries.insert(self.next, entries);
        self.next
    }
}
/// Only known committed slices count. Ambiguous outcomes stop the loop.
/// The kernel normally bounds each request to one slice; splitting is defensive.
pub async fn write_slices<F, Fut>(
    data: &[u8],
    offset: u64,
    max_io: usize,
    mut write: F,
) -> (usize, Option<ErrnoName>)
where
    F: FnMut(u64, Bytes, IdempotencyKey) -> Fut,
    Fut: Future<Output = Result<WriteResult, ClientError>>,
{
    let mut committed = 0;
    for slice in data.chunks(max_io) {
        let Some(at) = offset.checked_add(committed as u64) else {
            return (committed, Some(ErrnoName::EFBIG));
        };
        let mut result = None;
        for _ in 0..3 {
            let attempt = write(at, Bytes::copy_from_slice(slice), IdempotencyKey::new()).await;
            let conflict =
                attempt.as_ref().err().and_then(ClientError::code) == Some("generation_conflict");
            result = Some(attempt);
            if !conflict {
                break;
            }
        }
        match result.expect("one or more attempts") {
            Ok(result) if result.bytes_written == slice.len() as u64 => committed += slice.len(),
            Ok(_) => return (committed, Some(ErrnoName::EIO)),
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    "write slice did not establish publication"
                );
                if let ClientError::Api(e) = &error {
                    tracing::warn!(operation_id=?e.operation_id,trace_id=%e.trace_id,"write failure receipt");
                }
                return (committed, Some(errno_for(&error, CallClass::Mutation)));
            }
        }
    }
    (committed, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchorfs_client::error::ApiFailure;
    use std::sync::{Arc, Mutex};
    fn error(code: &str) -> ClientError {
        ClientError::Api(ApiFailure {
            status: 409,
            code: code.into(),
            message: String::new(),
            trace_id: String::new(),
            operation_id: None,
            retry_after_ms: None,
        })
    }
    #[tokio::test]
    async fn split_partial_and_ambiguous_writes() {
        for code in ["quota_exceeded", "outcome_unknown"] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let capture = calls.clone();
            let (done, errno) = write_slices(b"abcdefghijk", 10, 4, move |at, bytes, key| {
                let index = {
                    let mut calls = capture.lock().unwrap();
                    calls.push((at, bytes.to_vec(), key.as_str().to_owned()));
                    calls.len()
                };
                async move {
                    if index == 2 {
                        Err(error(code))
                    } else {
                        Ok(WriteResult {
                            operation_id: Uuid::new_v4(),
                            generation: 1,
                            bytes_written: bytes.len() as u64,
                            offset: at,
                        })
                    }
                }
            })
            .await;
            assert_eq!(done, 4);
            assert_eq!(
                errno,
                Some(if code == "quota_exceeded" {
                    ErrnoName::EDQUOT
                } else {
                    ErrnoName::EIO
                })
            );
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].0, 10);
            assert_eq!(calls[1].0, 14);
            assert_eq!(calls[1].1, b"efgh");
        }
    }
    #[tokio::test]
    async fn definitive_conflicts_rekey_at_most_three_times() {
        let keys = Arc::new(Mutex::new(Vec::new()));
        let capture = keys.clone();
        let result = write_slices(b"abc", 0, 4, move |_, _, key| {
            capture.lock().unwrap().push(key.as_str().to_owned());
            async { Err(error("generation_conflict")) }
        })
        .await;
        assert_eq!(result, (0, Some(ErrnoName::EIO)));
        let keys = keys.lock().unwrap();
        assert_eq!(keys.len(), 3);
        assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2]);
    }
    #[tokio::test]
    async fn successful_split_offsets_and_totals() {
        let offsets = Arc::new(Mutex::new(Vec::new()));
        let capture = offsets.clone();
        let result = write_slices(b"abcdefghij", 5, 4, move |at, bytes, _| {
            capture.lock().unwrap().push((at, bytes.len()));
            async move {
                Ok(WriteResult {
                    operation_id: Uuid::new_v4(),
                    generation: 1,
                    bytes_written: bytes.len() as u64,
                    offset: at,
                })
            }
        })
        .await;
        assert_eq!(result, (10, None));
        assert_eq!(*offsets.lock().unwrap(), vec![(5, 4), (9, 4), (13, 2)]);
    }
}
