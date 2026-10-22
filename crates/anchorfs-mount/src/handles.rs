use crate::errno::ErrnoName;
use anchorfs_client::{retry::IdempotencyKey, wire::WriteResult};
use bytes::Bytes;
use std::future::Future;
#[cfg(target_os = "linux")]
use std::{collections::BTreeMap, time::Instant};
#[cfg(any(target_os = "linux", test))]
use uuid::Uuid;
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) struct OpenFile {
    pub handle_id: Uuid,
    pub server_ino: i64,
    pub fenced: bool,
    pub renewed: Instant,
}
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct OpenFiles {
    next: u64,
    pub(crate) entries: BTreeMap<u64, OpenFile>,
}
#[cfg(target_os = "linux")]
impl OpenFiles {
    pub fn insert(&mut self, file: OpenFile) -> u64 {
        self.next += 1;
        self.entries.insert(self.next, file);
        self.next
    }
}
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(crate) struct DirEntrySnapshot {
    pub ino: u64,
    pub name: String,
    pub kind: anchorfs_client::wire::InodeKind,
}
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct OpenDirs {
    next: u64,
    pub(crate) entries: BTreeMap<u64, Vec<DirEntrySnapshot>>,
}
#[cfg(target_os = "linux")]
impl OpenDirs {
    pub fn insert(&mut self, entries: Vec<DirEntrySnapshot>) -> u64 {
        self.next += 1;
        self.entries.insert(self.next, entries);
        self.next
    }
}
/// The adapter owns fence/error policy. This portable algorithm only counts
/// committed slices and retries definitive generation conflicts under fresh keys.
pub enum Slice {
    Committed(WriteResult),
    Conflict,
    Failed(ErrnoName),
}
/// Kernel requests normally fit one slice; splitting is defensive.
pub async fn write_slices<F, Fut>(
    data: &[u8],
    offset: u64,
    max_io: usize,
    mut write: F,
) -> (usize, Option<ErrnoName>)
where
    F: FnMut(u64, Bytes, IdempotencyKey) -> Fut,
    Fut: Future<Output = Slice>,
{
    let mut committed = 0;
    for slice in data.chunks(max_io) {
        let Some(at) = offset.checked_add(committed as u64) else {
            return (committed, Some(ErrnoName::EFBIG));
        };
        let mut conflicts = 0;
        loop {
            match write(at, Bytes::copy_from_slice(slice), IdempotencyKey::new()).await {
                Slice::Committed(result) if result.bytes_written == slice.len() as u64 => {
                    committed += slice.len();
                    break;
                }
                Slice::Committed(_) => return (committed, Some(ErrnoName::EIO)),
                Slice::Failed(error) => return (committed, Some(error)),
                Slice::Conflict => {
                    conflicts += 1;
                    if conflicts == 3 {
                        return (committed, Some(ErrnoName::EIO));
                    }
                }
            }
        }
    }
    (committed, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
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
                        Slice::Failed(if code == "quota_exceeded" {
                            ErrnoName::EDQUOT
                        } else {
                            ErrnoName::EIO
                        })
                    } else {
                        Slice::Committed(WriteResult {
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
            async { Slice::Conflict }
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
                Slice::Committed(WriteResult {
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
