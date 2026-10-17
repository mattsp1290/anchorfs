pub mod platform;
pub mod profile;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MountError {
    #[error("mounting is supported only on Linux")]
    UnsupportedPlatform,
    #[error("only the strict stage 0 mount profile is supported")]
    UnsupportedProfile,
}
