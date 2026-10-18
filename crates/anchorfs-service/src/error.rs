use crate::dto::ErrorBody;
use anchorfs_core::ErrorCode;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[derive(Debug)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub trace_id: String,
    pub retry_after_ms: Option<u64>,
    pub operation_id: Option<anchorfs_core::OperationId>,
    pub token_presented: bool,
}
impl ApiError {
    pub fn new(code: ErrorCode, trace: impl Into<String>) -> Self {
        Self {
            code,
            message: code.as_str().to_owned(),
            trace_id: trace.into(),
            retry_after_ms: None,
            operation_id: None,
            token_presented: false,
        }
    }
    pub fn not_implemented(trace: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotImplemented, trace)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let token_presented = self.token_presented;
        let unauthenticated = self.code == ErrorCode::Unauthenticated;
        let mut response = (
            StatusCode::from_u16(self.code.status()).expect("static status"),
            Json(ErrorBody {
                code: self.code.into(),
                message: self.message,
                trace_id: self.trace_id,
                operation_id: self.operation_id.map(|id| id.0),
                retry_after_ms: self.retry_after_ms,
            }),
        )
            .into_response();
        if unauthenticated {
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static(if token_presented {
                    "Bearer error=\"invalid_token\""
                } else {
                    "Bearer"
                }),
            );
        }
        response
    }
}
pub fn map_rejection(status: StatusCode, trace: impl Into<String>) -> ApiError {
    let code = match status {
        StatusCode::PAYLOAD_TOO_LARGE => ErrorCode::PayloadTooLarge,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ErrorCode::UnsupportedMediaType,
        _ => ErrorCode::InvalidInput,
    };
    ApiError::new(code, trace)
}

impl From<anchorfs_persistence::authority::AuthorityError> for ApiError {
    fn from(error: anchorfs_persistence::authority::AuthorityError) -> Self {
        authority(error, "")
    }
}
pub fn authority(
    error: anchorfs_persistence::authority::AuthorityError,
    trace: impl Into<String>,
) -> ApiError {
    use anchorfs_persistence::authority::AuthorityError as A;
    let trace = trace.into();
    let code = match error {
        A::Unauthenticated => ErrorCode::Unauthenticated,
        A::WorkspaceBusy => ErrorCode::WorkspaceBusy,
        A::Fenced => ErrorCode::Fenced,
        A::Forbidden => ErrorCode::Forbidden,
        A::NotFound => ErrorCode::NotFound,
        A::InvalidInput => ErrorCode::InvalidInput,
        A::Database(error) => return database(error, trace),
    };
    let mut error = ApiError::new(code, trace);
    error.token_presented = true;
    error
}
pub fn database(error: sqlx::Error, trace: impl Into<String>) -> ApiError {
    let mut result = ApiError::new(ErrorCode::DependencyUnavailable, trace);
    if error
        .as_database_error()
        .and_then(|e| e.code())
        .is_some_and(|code| matches!(code.as_ref(), "40P01" | "40001" | "55P03" | "57014"))
    {
        result.retry_after_ms = Some(100);
    }
    result
}
impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        database(error, "")
    }
}
impl From<anchorfs_persistence::sessions::SessionError> for ApiError {
    fn from(error: anchorfs_persistence::sessions::SessionError) -> Self {
        use anchorfs_persistence::sessions::SessionError;
        match error {
            SessionError::Authority(error) => error.into(),
            SessionError::Limit => ApiError::new(ErrorCode::QuotaExceeded, ""),
            SessionError::Database(error) => error.into(),
        }
    }
}
impl From<anchorfs_persistence::quota::QuotaError> for ApiError {
    fn from(error: anchorfs_persistence::quota::QuotaError) -> Self {
        match error {
            anchorfs_persistence::quota::QuotaError::Exceeded => {
                Self::new(ErrorCode::QuotaExceeded, "")
            }
            anchorfs_persistence::quota::QuotaError::Database(error) => error.into(),
        }
    }
}
impl From<anchorfs_persistence::namespace::NamespaceError> for ApiError {
    fn from(error: anchorfs_persistence::namespace::NamespaceError) -> Self {
        use anchorfs_persistence::namespace::NamespaceError as N;
        Self::new(
            match error {
                N::NotFound => ErrorCode::NotFound,
                N::NotDirectory => ErrorCode::NotDirectory,
                N::IsDirectory => ErrorCode::IsDirectory,
                N::DirectoryNotEmpty => ErrorCode::DirectoryNotEmpty,
                N::Exists => ErrorCode::Exists,
                N::Forbidden => ErrorCode::Forbidden,
                N::InvalidInput => ErrorCode::InvalidInput,
                N::NotImplemented => ErrorCode::NotImplemented,
                N::Quota(error) => return error.into(),
                N::Database(error) => return error.into(),
            },
            "",
        )
    }
}
