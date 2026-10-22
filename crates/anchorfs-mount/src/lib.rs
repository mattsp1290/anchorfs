pub mod config;
pub mod errno;
pub mod handles;
pub mod inodes;
pub mod platform;
pub mod profile;
#[cfg(target_os = "linux")]
pub mod runtime;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MountError {
    #[error("mounting is supported only on Linux")]
    UnsupportedPlatform,
    #[error("only the strict mount profile is supported")]
    UnsupportedProfile,
    #[error("configuration: {0}")]
    Config(String),
    #[error("authorization: {0}")]
    Authorization(String),
    #[error("service unavailable during mount establishment: {0}")]
    Unavailable(String),
    #[error("mount ({kind}): {message}")]
    Mount { kind: String, message: String },
    #[error("mount identity fenced")]
    Fenced,
}

impl MountError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::UnsupportedPlatform | Self::UnsupportedProfile | Self::Config(_) => 2,
            Self::Authorization(_) => 3,
            Self::Unavailable(_) | Self::Mount { .. } => 4,
            Self::Fenced => 5,
        }
    }
}
