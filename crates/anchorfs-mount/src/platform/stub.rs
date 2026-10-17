use crate::{MountError, profile::MountProfile};
pub fn platform_supported() -> bool {
    false
}
pub fn mount_options(_profile: &MountProfile) -> Result<Vec<String>, MountError> {
    Err(MountError::UnsupportedPlatform)
}
