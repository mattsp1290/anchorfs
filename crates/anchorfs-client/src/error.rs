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
#[derive(Debug, Clone, thiserror::Error)]
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
    /// Publication cannot be excluded, even if a later replay was rejected.
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Self::Transport(_) | Self::Deadline | Self::Decode(_))
            || matches!(
                self.code(),
                Some("outcome_unknown" | "operation_indeterminate")
            )
    }
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
