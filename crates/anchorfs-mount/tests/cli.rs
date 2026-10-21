use anchorfs_mount::profile::MountProfile;
use std::process::Command;
#[test]
fn prints_profile_on_every_platform() {
    let output = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"))
        .arg("--print-profile")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<MountProfile>(&output.stdout).unwrap(),
        MountProfile::strict()
    );
}
#[test]
fn mount_requires_valid_arguments_and_credentials() {
    let output = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"))
        .args(["mount", "/synthetic/mount", "--workspace", "synthetic"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
}
