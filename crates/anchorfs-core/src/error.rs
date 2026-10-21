/// Stable wire message distinguishing an expired publication reservation from authority fencing.
pub const RESERVATION_EXPIRED: &str = "reservation_expired";

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
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
impl ErrorCode {
    pub const ALL: &'static [Self] = &[
        Self::InvalidInput,
        Self::NameTooLong,
        Self::Unauthenticated,
        Self::Forbidden,
        Self::NotFound,
        Self::MethodNotAllowed,
        Self::Exists,
        Self::GenerationConflict,
        Self::IdempotencyMismatch,
        Self::WorkspaceBusy,
        Self::Fenced,
        Self::OperationIndeterminate,
        Self::OperationFailed,
        Self::NotDirectory,
        Self::IsDirectory,
        Self::DirectoryNotEmpty,
        Self::StaleHandle,
        Self::PayloadTooLarge,
        Self::FileTooLarge,
        Self::UnsupportedMediaType,
        Self::RateLimited,
        Self::QuotaExceeded,
        Self::CapacityExhausted,
        Self::Unsupported,
        Self::NotImplemented,
        Self::DependencyUnavailable,
        Self::OutcomeUnknown,
        Self::UnsafeDatabaseSettings,
        Self::SchemaVersionMismatch,
    ];
    pub fn status(self) -> u16 {
        match self {
            Self::InvalidInput => 400,
            Self::NameTooLong => 400,
            Self::Unauthenticated => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::Exists => 409,
            Self::GenerationConflict => 409,
            Self::IdempotencyMismatch => 409,
            Self::WorkspaceBusy => 409,
            Self::Fenced => 409,
            Self::OperationIndeterminate => 409,
            Self::OperationFailed => 409,
            Self::NotDirectory => 409,
            Self::IsDirectory => 409,
            Self::DirectoryNotEmpty => 409,
            Self::StaleHandle => 410,
            Self::PayloadTooLarge => 413,
            Self::FileTooLarge => 413,
            Self::UnsupportedMediaType => 415,
            Self::RateLimited => 429,
            Self::QuotaExceeded => 507,
            Self::CapacityExhausted => 507,
            Self::Unsupported => 501,
            Self::NotImplemented => 501,
            Self::DependencyUnavailable => 503,
            Self::OutcomeUnknown => 503,
            Self::UnsafeDatabaseSettings => 503,
            Self::SchemaVersionMismatch => 503,
        }
    }
    pub fn errno_hint(self) -> &'static str {
        match self {
            Self::InvalidInput => "EINVAL",
            Self::NameTooLong => "ENAMETOOLONG",
            Self::Unauthenticated => "EACCES",
            Self::Forbidden => "EACCES",
            Self::NotFound => "ENOENT",
            Self::MethodNotAllowed => "EOPNOTSUPP",
            Self::Exists => "EEXIST",
            Self::GenerationConflict => {
                "internal retry for unconditional writes; conflict for conditional"
            }
            Self::IdempotencyMismatch => "EIO",
            Self::WorkspaceBusy => "EBUSY",
            Self::Fenced => "ESTALE",
            Self::OperationIndeterminate => "EIO",
            Self::OperationFailed => "EIO",
            Self::NotDirectory => "ENOTDIR",
            Self::IsDirectory => "EISDIR",
            Self::DirectoryNotEmpty => "ENOTEMPTY",
            Self::StaleHandle => "ESTALE",
            Self::PayloadTooLarge => "EINVAL",
            Self::FileTooLarge => "EFBIG",
            Self::UnsupportedMediaType => "EINVAL",
            Self::RateLimited => "EAGAIN",
            Self::QuotaExceeded => "EDQUOT",
            Self::CapacityExhausted => "ENOSPC",
            Self::Unsupported => "EOPNOTSUPP",
            Self::NotImplemented => "EOPNOTSUPP",
            Self::DependencyUnavailable => "EIO / ETIMEDOUT per call",
            Self::OutcomeUnknown => "resolve receipt before retry",
            Self::UnsafeDatabaseSettings => "n/a",
            Self::SchemaVersionMismatch => "n/a",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::NameTooLong => "name_too_long",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::Exists => "exists",
            Self::GenerationConflict => "generation_conflict",
            Self::IdempotencyMismatch => "idempotency_mismatch",
            Self::WorkspaceBusy => "workspace_busy",
            Self::Fenced => "fenced",
            Self::OperationIndeterminate => "operation_indeterminate",
            Self::OperationFailed => "operation_failed",
            Self::NotDirectory => "not_directory",
            Self::IsDirectory => "is_directory",
            Self::DirectoryNotEmpty => "directory_not_empty",
            Self::StaleHandle => "stale_handle",
            Self::PayloadTooLarge => "payload_too_large",
            Self::FileTooLarge => "file_too_large",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::RateLimited => "rate_limited",
            Self::QuotaExceeded => "quota_exceeded",
            Self::CapacityExhausted => "capacity_exhausted",
            Self::Unsupported => "unsupported",
            Self::NotImplemented => "not_implemented",
            Self::DependencyUnavailable => "dependency_unavailable",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::UnsafeDatabaseSettings => "unsafe_database_settings",
            Self::SchemaVersionMismatch => "schema_version_mismatch",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<crate::OperationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_codes_unique_and_total() {
        let names: std::collections::BTreeSet<_> =
            ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(names.len(), ErrorCode::ALL.len());
        for code in ErrorCode::ALL {
            assert!((400..600).contains(&code.status()));
            assert!(!code.errno_hint().is_empty());
        }
    }
}
