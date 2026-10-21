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
    #[error("only the strict stage 0 mount profile is supported")]
    UnsupportedProfile,
    #[error("configuration: {0}")]
    Config(String),
    #[error("authorization: {0}")]
    Authorization(String),
    #[error("mount ({kind}): {message}")]
    Mount { kind: String, message: String },
    #[error("mount identity fenced")]
    Fenced,
}
