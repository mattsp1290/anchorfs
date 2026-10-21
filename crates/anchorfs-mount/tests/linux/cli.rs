use super::{fixture::*, mountpoint::*};
use std::{fs, process::Command};
#[test]
fn cli_exit_codes_and_secret_sources() {
    let _serial = serial();
    let f = Fixture::new();
    let path = std::env::temp_dir().join(format!("anchorfs-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&path).unwrap();
    let run = |allow_plaintext: bool, credential: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"));
        cmd.arg("mount")
            .arg(&path)
            .arg("--endpoint")
            .arg(&f.endpoint)
            .arg("--workspace")
            .arg(f.workspace.to_string())
            .env_remove("ANCHORFS_MOUNT_CREDENTIAL")
            .env_remove("ANCHORFS_MOUNT_WORKFLOW_TOKEN");
        if credential {
            cmd.env("ANCHORFS_MOUNT_CREDENTIAL", &f.worker.bearer);
        }
        if allow_plaintext {
            cmd.arg("--allow-plaintext");
        }
        cmd.output().unwrap()
    };
    assert_eq!(run(true, false).status.code(), Some(2));
    assert_eq!(run(false, true).status.code(), Some(2));
    let worker = run(true, true);
    assert_eq!(worker.status.code(), Some(3));
    assert!(
        String::from_utf8(worker.stderr)
            .unwrap()
            .contains("host-minted session")
    );
    let mut m = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let cmdline = fs::read(format!(
        "/proc/{}/cmdline",
        m.process.as_ref().unwrap().id()
    ))
    .unwrap();
    assert!(!String::from_utf8_lossy(&cmdline).contains(&f.worker.bearer));
    m.stop();
    let profile = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"))
        .arg("--print-profile")
        .output()
        .unwrap();
    assert!(profile.status.success());
    let _: anchorfs_mount::profile::MountProfile = serde_json::from_slice(&profile.stdout).unwrap();
    fs::remove_dir(path).unwrap();
    f.invariants();
}

#[test]
fn startup_failure_revokes_self_minted_session() {
    let _serial = serial();
    let f = Fixture::new();
    let address = f.endpoint.strip_prefix("http://").unwrap().parse().unwrap();
    let relay = f.run(super::relay::ApiProxy::start(address));
    relay.fail_requests(
        &format!("GET /v1/workspaces/{} HTTP", f.workspace),
        3,
        503,
        "dependency_unavailable",
    );
    let path = std::env::temp_dir().join(format!("anchorfs-startup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&path).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"))
        .arg("mount")
        .arg(&path)
        .arg("--endpoint")
        .arg(&relay.endpoint)
        .arg("--workspace")
        .arg(f.workspace.to_string())
        .arg("--allow-plaintext")
        .env("ANCHORFS_MOUNT_CREDENTIAL", &f.host.bearer)
        .env_remove("ANCHORFS_MOUNT_WORKFLOW_TOKEN")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(3));
    assert_eq!(f.count("session_revoke"), 1);
    fs::remove_dir(path).unwrap();
    f.invariants();
}
