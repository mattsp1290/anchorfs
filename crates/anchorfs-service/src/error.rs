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
}
impl ApiError {
    pub fn new(code: ErrorCode, trace: impl Into<String>) -> Self {
        Self {
            code,
            message: code.as_str().to_owned(),
            trace_id: trace.into(),
            retry_after_ms: None,
        }
    }
    pub fn not_implemented(trace: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotImplemented, trace)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::from_u16(self.code.status()).expect("static status"),
            Json(ErrorBody {
                code: self.code.into(),
                message: self.message,
                trace_id: self.trace_id,
                operation_id: None,
                retry_after_ms: self.retry_after_ms,
            }),
        )
            .into_response()
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
