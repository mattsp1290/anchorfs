#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(target_os = "linux")]
pub use linux::{AnchorFs, mount_options, platform_supported};
#[cfg(not(target_os = "linux"))]
pub use stub::{mount_options, platform_supported};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MountError, profile::MountProfile};
    #[test]
    fn platform_contract() {
        // Compile-check the same signature on every supported compilation target.
        let options: fn(&MountProfile) -> Result<Vec<String>, MountError> = mount_options;
        if cfg!(target_os = "linux") {
            assert!(platform_supported());
            assert_eq!(
                options(&MountProfile::strict()).unwrap(),
                vec!["default_permissions"]
            );
        } else {
            assert!(!platform_supported());
            assert_eq!(
                options(&MountProfile::strict()),
                Err(MountError::UnsupportedPlatform)
            );
        }
    }
}
