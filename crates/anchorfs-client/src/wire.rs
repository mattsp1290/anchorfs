//! Independent client wire shapes derived from AnchorFS service DTOs.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InodeAttributes {
    pub inode_id: i64,
    pub generation: i64,
    pub kind: InodeKind,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    #[serde(with = "timestamp")]
    pub atime: time::OffsetDateTime,
    #[serde(with = "timestamp")]
    pub mtime: time::OffsetDateTime,
    #[serde(with = "timestamp")]
    pub ctime: time::OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub attributes: InodeAttributes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryPage {
    pub namespace_generation: i64,
    pub entries: Vec<DirectoryEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleGrant {
    pub operation_id: Uuid,
    pub handle_id: Uuid,
    pub attributes: InodeAttributes,
    pub access_read: bool,
    pub access_write: bool,
    #[serde(with = "timestamp")]
    pub expires_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGrant {
    pub operation_id: Uuid,
    pub session_id: Uuid,
    pub session_epoch: i64,
    pub credential_epoch: i64,
    pub bound_mutation_epoch: Option<i64>,
    #[serde(with = "timestamp")]
    pub expires_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResult {
    pub operation_id: Uuid,
    pub generation: i64,
    pub bytes_written: u64,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    pub operation_id: Uuid,
    pub inode_id: Option<i64>,
    pub generation: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymlinkContents {
    pub target: String,
    pub generation: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub workflow: Option<WorkflowStatus>,
    pub workspace_id: Uuid,
    pub root: InodeAttributes,
    pub authorization_epoch: i64,
    pub mutation_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub owner_credential_id: Uuid,
    pub mutation_epoch: i64,
    #[serde(with = "timestamp")]
    pub expires_at: time::OffsetDateTime,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStatus {
    pub operation_id: Uuid,
    pub state: OperationState,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Pending,
    Committed,
    Failed,
    Indeterminate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InodeKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenHandle {
    pub path: String,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAttributes {
    /// Omit to select the workspace root.
    pub path: Option<String>,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// Explicit RFC3339 time requires the inode owner or a workspace administrator.
    /// Omission leaves it unchanged. PostgreSQL retains microsecond precision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atime: Option<String>,
    /// Explicit RFC3339 time requires the inode owner or a workspace administrator.
    /// Omission leaves it unchanged. PostgreSQL retains microsecond precision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<String>,
}

mod timestamp {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    pub fn serialize<S: Serializer>(
        value: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OffsetDateTime, D::Error> {
        OffsetDateTime::parse(&String::deserialize(deserializer)?, &Rfc3339)
            .map_err(serde::de::Error::custom)
    }
}
