use anchorfs_core::ErrorCode;
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

/// Known protocol codes are parsed once. Unknown future codes retain their raw
/// spelling for diagnostics and conservatively map to EIO at a mount boundary.
#[derive(Debug, Clone)]
pub enum ApiCode {
    Known(ErrorCode),
    Unknown(String),
}
impl ApiCode {
    pub fn kind(&self) -> Option<ErrorCode> {
        match self {
            Self::Known(code) => Some(*code),
            Self::Unknown(_) => None,
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Self::Known(code) => code.as_str(),
            Self::Unknown(raw) => raw,
        }
    }
}
impl From<ErrorCode> for ApiCode {
    fn from(code: ErrorCode) -> Self {
        Self::Known(code)
    }
}
impl From<String> for ApiCode {
    fn from(raw: String) -> Self {
        let value = serde::de::value::StrDeserializer::<serde::de::value::Error>::new(&raw);
        match ErrorCode::deserialize(value) {
            Ok(code) => Self::Known(code),
            Err(_) => Self::Unknown(raw),
        }
    }
}
impl From<&str> for ApiCode {
    fn from(raw: &str) -> Self {
        Self::from(raw.to_owned())
    }
}
impl<'de> Deserialize<'de> for ApiCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self::from)
    }
}
impl std::fmt::Display for ApiCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiFailure {
    #[serde(skip)]
    pub status: u16,
    pub code: ApiCode,
    pub message: String,
    pub trace_id: String,
    pub operation_id: Option<Uuid>,
    pub retry_after_ms: Option<u64>,
}
impl ApiFailure {
    pub fn errno_hint(&self) -> Option<&'static str> {
        self.code.kind().map(|code| code.errno_hint())
    }
}
#[derive(Debug, Clone, thiserror::Error)]
pub enum ClientError {
    #[error("API {0:?}")]
    Api(ApiFailure),
    #[error("mutation outcome is ambiguous: {0}")]
    Ambiguous(Box<ClientError>),
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
        matches!(
            self,
            Self::Transport(_) | Self::Deadline | Self::Decode(_) | Self::Ambiguous(_)
        ) || matches!(
            self.kind(),
            Some(ErrorCode::OutcomeUnknown | ErrorCode::OperationIndeterminate)
        )
    }
    pub fn kind(&self) -> Option<ErrorCode> {
        match self {
            Self::Api(e) => e.code.kind(),
            _ => None,
        }
    }
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api(e) => Some(e.code.as_str()),
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
