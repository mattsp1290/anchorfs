use crate::{MountError, profile::MountProfile};
use fuser::{Filesystem, KernelConfig, MountOption, Request};

pub struct AnchorFs;
impl Filesystem for AnchorFs {
    fn init(&mut self, _request: &Request, _config: &mut KernelConfig) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn platform_supported() -> bool {
    true
}
pub fn mount_options(profile: &MountProfile) -> Result<Vec<String>, MountError> {
    if profile != &MountProfile::strict() {
        return Err(MountError::UnsupportedProfile);
    }
    // fuser exposes no public string renderer. Render only the selected option;
    // direct I/O and zero TTLs belong to replies, not kernel mount flags.
    let options = [MountOption::DefaultPermissions];
    Ok(options
        .iter()
        .map(|option| match option {
            MountOption::DefaultPermissions => "default_permissions".to_owned(),
            _ => unreachable!("only DefaultPermissions is selected"),
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_relaxed_profile() {
        let mut profile = MountProfile::strict();
        profile.writeback_cache = true;
        assert_eq!(mount_options(&profile), Err(MountError::UnsupportedProfile));
    }
}
