use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiFailure {
    #[serde(skip)]
    pub status: u16,
    pub code: String,
    pub message: String,
    pub trace_id: String,
    pub operation_id: Option<Uuid>,
    pub retry_after_ms: Option<u64>,
}
impl ApiFailure {
    pub fn errno_hint(&self) -> Option<&'static str> {
        anchorfs_core::ErrorCode::ALL
            .iter()
            .find(|code| code.as_str() == self.code)
            .map(|code| code.errno_hint())
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("API {0:?}")]
    Api(ApiFailure),
    #[error("transport: {0}")]
    Transport(String),
    #[error("client deadline exceeded")]
    Deadline,
    #[error("response decode: {0}")]
    Decode(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}
impl ClientError {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api(e) => Some(&e.code),
            _ => None,
        }
    }
    pub(crate) fn transport(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Deadline
        } else {
            Self::Transport(error.without_url().to_string())
        }
    }
}
