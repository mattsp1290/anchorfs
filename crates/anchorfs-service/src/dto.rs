use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCodeDto {
    InvalidInput,
    NameTooLong,
    Unauthenticated,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Exists,
    GenerationConflict,
    IdempotencyMismatch,
    WorkspaceBusy,
    Fenced,
    OperationIndeterminate,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    StaleHandle,
    PayloadTooLarge,
    FileTooLarge,
    UnsupportedMediaType,
    RateLimited,
    QuotaExceeded,
    CapacityExhausted,
    Unsupported,
    NotImplemented,
    DependencyUnavailable,
    OutcomeUnknown,
    UnsafeDatabaseSettings,
    SchemaVersionMismatch,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: ErrorCodeDto,
    pub message: String,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}
impl From<anchorfs_core::ErrorCode> for ErrorCodeDto {
    fn from(code: anchorfs_core::ErrorCode) -> Self {
        match code {
            anchorfs_core::ErrorCode::InvalidInput => Self::InvalidInput,
            anchorfs_core::ErrorCode::NameTooLong => Self::NameTooLong,
            anchorfs_core::ErrorCode::Unauthenticated => Self::Unauthenticated,
            anchorfs_core::ErrorCode::Forbidden => Self::Forbidden,
            anchorfs_core::ErrorCode::NotFound => Self::NotFound,
            anchorfs_core::ErrorCode::MethodNotAllowed => Self::MethodNotAllowed,
            anchorfs_core::ErrorCode::Exists => Self::Exists,
            anchorfs_core::ErrorCode::GenerationConflict => Self::GenerationConflict,
            anchorfs_core::ErrorCode::IdempotencyMismatch => Self::IdempotencyMismatch,
            anchorfs_core::ErrorCode::WorkspaceBusy => Self::WorkspaceBusy,
            anchorfs_core::ErrorCode::Fenced => Self::Fenced,
            anchorfs_core::ErrorCode::OperationIndeterminate => Self::OperationIndeterminate,
            anchorfs_core::ErrorCode::NotDirectory => Self::NotDirectory,
            anchorfs_core::ErrorCode::IsDirectory => Self::IsDirectory,
            anchorfs_core::ErrorCode::DirectoryNotEmpty => Self::DirectoryNotEmpty,
            anchorfs_core::ErrorCode::StaleHandle => Self::StaleHandle,
            anchorfs_core::ErrorCode::PayloadTooLarge => Self::PayloadTooLarge,
            anchorfs_core::ErrorCode::FileTooLarge => Self::FileTooLarge,
            anchorfs_core::ErrorCode::UnsupportedMediaType => Self::UnsupportedMediaType,
            anchorfs_core::ErrorCode::RateLimited => Self::RateLimited,
            anchorfs_core::ErrorCode::QuotaExceeded => Self::QuotaExceeded,
            anchorfs_core::ErrorCode::CapacityExhausted => Self::CapacityExhausted,
            anchorfs_core::ErrorCode::Unsupported => Self::Unsupported,
            anchorfs_core::ErrorCode::NotImplemented => Self::NotImplemented,
            anchorfs_core::ErrorCode::DependencyUnavailable => Self::DependencyUnavailable,
            anchorfs_core::ErrorCode::OutcomeUnknown => Self::OutcomeUnknown,
            anchorfs_core::ErrorCode::UnsafeDatabaseSettings => Self::UnsafeDatabaseSettings,
            anchorfs_core::ErrorCode::SchemaVersionMismatch => Self::SchemaVersionMismatch,
        }
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InodeAttributes {
    pub inode_id: i64,
    pub generation: i64,
    pub kind: String,
    pub size: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub atime: String,
    pub mtime: String,
    pub ctime: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DirectoryEntry {
    pub name: String,
    pub attributes: InodeAttributes,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DirectoryPage {
    pub entries: Vec<DirectoryEntry>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationStatus {
    pub operation_id: Uuid,
    pub state: String,
    pub result: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkspaceIdParam {
    pub workspace_id: Uuid,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub logical_bytes_limit: u64,
    pub logical_inodes_limit: u64,
    pub physical_bytes_limit: u64,
    pub staging_bytes_limit: u64,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Administrator,
    Host,
    Worker,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRole {
    Read,
    Write,
    Admin,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCredential {
    pub workspace_id: Option<Uuid>,
    pub kind: CredentialKind,
    pub role: CredentialRole,
    pub uid: u32,
    pub gid: u32,
    pub expires_at: Option<String>,
    pub max_reservation_bytes: u64,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSession {
    pub credential_id: Uuid,
    pub bind_to_workflow_lease: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct RebindSession {
    pub workflow_lease_token: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDirectory {
    pub path: String,
    pub mode: u32,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct OpenHandle {
    pub path: String,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub mode: u32,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct Truncate {
    pub size: u64,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct PathBody {
    pub path: String,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLink {
    pub existing_path: String,
    pub new_path: String,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSymlink {
    pub path: String,
    pub target: String,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct Rename {
    pub from: String,
    pub to: String,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetAttributes {
    pub path: String,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: String,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Deserialize)]
pub struct ReadQuery {
    pub offset: u64,
    pub length: u32,
    pub read_pin: Option<Uuid>,
}
#[derive(Debug, Deserialize)]
pub struct WriteQuery {
    pub offset: u64,
}
#[derive(Debug, Deserialize)]
pub struct ReplaceQuery {
    pub path: String,
    pub expected_generation: i64,
}
#[derive(Debug, Deserialize)]
pub struct OperationQuery {
    pub idempotency_key: String,
}
