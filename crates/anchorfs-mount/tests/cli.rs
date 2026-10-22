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

#[cfg(not(target_os = "linux"))]
#[test]
fn valid_mount_configuration_reports_linux_requirement() {
    let output = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"))
        .args([
            "mount",
            "/synthetic/mount",
            "--endpoint",
            "https://localhost:1234",
            "--workspace",
            "00000000-0000-4000-8000-000000000001",
        ])
        .env(
            "ANCHORFS_MOUNT_CREDENTIAL",
            format!("00000000-0000-4000-8000-000000000002.{}", "ab".repeat(32)),
        )
        .env_remove("ANCHORFS_MOUNT_WORKFLOW_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("mounting is supported only on Linux")
    );
}
