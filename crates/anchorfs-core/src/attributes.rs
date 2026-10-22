use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// A normalized inode metadata change, shared by client, service and persistence.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AttributeUpdate {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub atime: Option<OffsetDateTime>,
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub mtime: Option<OffsetDateTime>,
}
impl AttributeUpdate {
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.uid.is_none()
            && self.gid.is_none()
            && self.atime.is_none()
            && self.mtime.is_none()
    }
}
