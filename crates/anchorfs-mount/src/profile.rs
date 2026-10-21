use serde::{Deserialize, Serialize};
use std::time::Duration;
/// Strict profile; shared writable mmap is unsupported. `default_permissions` is a mount
/// option and auto-cache is never enabled. Direct I/O is an open-time reply
/// setting; zero TTLs are applied to lookup/getattr replies.
/// Writeback caching must remain disabled during FUSE initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountProfile {
    pub direct_io: bool,
    pub writeback_cache: bool,
    pub attr_ttl: Duration,
    pub entry_ttl: Duration,
    pub negative_ttl: Duration,
    pub allow_mmap: bool,
    pub default_permissions: bool,
}
impl MountProfile {
    pub fn strict() -> Self {
        Self {
            direct_io: true,
            writeback_cache: false,
            attr_ttl: Duration::ZERO,
            entry_ttl: Duration::ZERO,
            negative_ttl: Duration::ZERO,
            allow_mmap: false,
            default_permissions: true,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_contract() {
        assert_eq!(
            serde_json::to_value(MountProfile::strict()).unwrap(),
            serde_json::json!({"direct_io":true,"writeback_cache":false,"attr_ttl":{"secs":0,"nanos":0},"entry_ttl":{"secs":0,"nanos":0},"negative_ttl":{"secs":0,"nanos":0},"allow_mmap":false,"default_permissions":true})
        );
    }
}
