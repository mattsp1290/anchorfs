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
    OperationFailed,
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
            anchorfs_core::ErrorCode::OperationFailed => Self::OperationFailed,
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
    pub kind: InodeKindDto,
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
    pub namespace_generation: i64,
    pub entries: Vec<DirectoryEntry>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationStatus {
    pub operation_id: Uuid,
    pub state: OperationStateDto,
    pub result: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct WorkspaceIdParam {
    pub workspace_id: Uuid,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub root_uid: u32,
    pub root_gid: u32,
    pub root_mode: Option<u32>,
    pub logical_bytes_limit: u64,
    pub logical_inodes_limit: u64,
    pub physical_bytes_limit: u64,
    pub staging_bytes_limit: u64,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Administrator,
    Host,
    Worker,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRole {
    Read,
    Write,
    Admin,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateCredential {
    pub workspace_id: Option<Uuid>,
    pub kind: CredentialKind,
    pub role: CredentialRole,
    pub uid: u32,
    pub gid: u32,
    pub expires_at: Option<String>,
    pub max_reservation_bytes: u64,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct CreateSession {
    pub credential_id: Uuid,
    pub bind_to_workflow_lease: Option<String>,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct RebindSession {
    pub workflow_lease_token: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateDirectory {
    pub path: String,
    pub mode: u32,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OpenHandle {
    pub path: String,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub mode: Option<u32>,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct Truncate {
    pub size: u64,
}
#[derive(Debug, Deserialize, ToSchema)]
pub struct PathBody {
    pub path: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateLink {
    pub existing_path: String,
    pub new_path: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateSymlink {
    pub path: String,
    pub target: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Rename {
    pub from: String,
    pub to: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
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

/// Absent path selects the workspace root. A supplied path must remain nonempty and relative.
#[derive(Debug, Deserialize)]
pub struct LookupQuery {
    pub path: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceInfo {
    pub workflow: Option<WorkflowStatus>,
    pub workspace_id: Uuid,
    pub root: InodeAttributes,
    pub authorization_epoch: i64,
    pub mutation_epoch: i64,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreatedWorkspace {
    pub operation_id: Uuid,
    pub workspace: WorkspaceInfo,
}
/// Secret is returned only on issue/rotation; never on inspection or in logs.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct IssuedCredential {
    pub operation_id: Uuid,
    pub credential_id: Uuid,
    /// Present on the initial response; receipt replays return null.
    pub secret: Option<String>,
    pub credential_epoch: i64,
    pub expires_at: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionGrant {
    pub operation_id: Uuid,
    pub session_id: Uuid,
    pub session_epoch: i64,
    pub credential_epoch: i64,
    pub bound_mutation_epoch: Option<i64>,
    pub expires_at: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HandleGrant {
    pub operation_id: Uuid,
    pub handle_id: Uuid,
    pub attributes: InodeAttributes,
    pub access_read: bool,
    pub access_write: bool,
    pub expires_at: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadPinGrant {
    pub operation_id: Uuid,
    pub read_pin_id: Uuid,
    pub manifest_id: Uuid,
    pub generation: i64,
    pub size: u64,
    pub expires_at: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct WorkflowGrant {
    pub operation_id: Uuid,
    /// Present only on initial acquire/takeover; renewals and receipt replays return null.
    /// A replayed grant is not a usable token.
    pub token: Option<String>,
    pub mutation_epoch: i64,
    pub expires_at: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MutationResult {
    pub operation_id: Uuid,
    pub inode_id: Option<i64>,
    pub generation: Option<i64>,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteResult {
    pub operation_id: Uuid,
    pub generation: i64,
    pub bytes_written: u64,
    pub offset: u64,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SymlinkContents {
    pub target: String,
    pub generation: i64,
}

/// Raw bytes on the wire, rather than a JSON array of integers.
pub struct BinaryContent;
impl utoipa::PartialSchema for BinaryContent {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, SchemaFormat, Type};
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary)))
            .build()
            .into()
    }
}
impl ToSchema for BinaryContent {}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InodeKindDto {
    File,
    Directory,
    Symlink,
}
impl From<anchorfs_core::InodeKind> for InodeKindDto {
    fn from(kind: anchorfs_core::InodeKind) -> Self {
        match kind {
            anchorfs_core::InodeKind::File => Self::File,
            anchorfs_core::InodeKind::Directory => Self::Directory,
            anchorfs_core::InodeKind::Symlink => Self::Symlink,
        }
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationStateDto {
    Pending,
    Committed,
    Failed,
    Indeterminate,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowStatus {
    pub owner_credential_id: Uuid,
    pub mutation_epoch: i64,
    pub expires_at: String,
    pub active: bool,
}

impl std::fmt::Debug for CreateSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateSession")
            .field("bind_to_workflow_lease", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RebindSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RebindSession")
            .field("workflow_lease_token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for WorkflowGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowGrant")
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod lease_debug_tests {
    use super::*;
    #[test]
    fn request_debug_redacts_lease_tokens() {
        let token = "ab".repeat(32);
        let values = [
            format!(
                "{:?}",
                CreateSession {
                    credential_id: Uuid::nil(),
                    bind_to_workflow_lease: Some(token.clone())
                }
            ),
            format!(
                "{:?}",
                RebindSession {
                    workflow_lease_token: Some(token.clone())
                }
            ),
            format!(
                "{:?}",
                WorkflowGrant {
                    operation_id: Uuid::nil(),
                    token: Some(token.clone()),
                    mutation_epoch: 2,
                    expires_at: "synthetic".into()
                }
            ),
        ];
        for value in values {
            assert!(value.contains("[redacted]"));
            assert!(!value.contains(&token));
        }
    }
}
