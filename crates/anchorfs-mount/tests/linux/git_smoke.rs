use super::{fixture::*, mountpoint::*};
use serde_json::json;
use std::{fs, os::fd::AsRawFd, process::Command, time::Instant};
#[test]
fn git_smoke_under_lease() {
    let _serial = serial();
    let f = Fixture::new();
    let lease = f.raw(
        &f.host,
        Some(&f.host_session),
        "POST",
        "/workflow-lease",
        Some(json!({})),
    );
    let session = f.raw(
        &f.host,
        None,
        "POST",
        "/sessions",
        Some(json!({"credential_id":f.worker.id,"bind_to_workflow_lease":lease["token"]})),
    );
    let m = TempMount::process(
        &f,
        &f.worker,
        Some(&session),
        Some((
            lease["token"].as_str().unwrap(),
            lease["mutation_epoch"].as_i64().unwrap(),
        )),
        1048576,
    );
    let git = |args: &[&str]| {
        let start = Instant::now();
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=AnchorFS Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&m.path)
            .output()
            .unwrap();
        eprintln!(
            "git {args:?}: {:?} in {:?}; stderr: {}",
            output.status.code(),
            start.elapsed(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init"]);
    fs::write(m.path.join("note.txt"), b"first commit\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "first"]);
    assert!(git(&["status", "--porcelain"]).trim().is_empty());
    assert_eq!(git(&["log", "--oneline"]).lines().count(), 1);
    git(&["checkout", "-b", "x"]);
    fs::write(m.path.join("note.txt"), b"second commit\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "second"]);
    git(&["switch", "-"]);
    assert!(git(&["status", "--porcelain"]).trim().is_empty());
    assert_eq!(
        fs::read(m.path.join("note.txt")).unwrap(),
        b"first commit\n"
    );
    let index = fs::File::open(m.path.join(".git/index")).unwrap();
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            index.as_raw_fd(),
            0,
        );
        eprintln!(
            "Git index read-only private mmap succeeded: {}",
            ptr != libc::MAP_FAILED
        );
        if ptr != libc::MAP_FAILED {
            libc::munmap(ptr, 4096);
        }
    }
    drop(index);
    // Independent API reads use the same bound session proof while the lease is live.
    let client = native(&f.endpoint, f.workspace, &f.worker, Some(&session));
    let mut authority = client.authority();
    authority.workflow = Some(anchorfs_client::auth::WorkflowAuthority {
        token: anchorfs_core::Secret::new(lease["token"].as_str().unwrap().into()),
        epoch: lease["mutation_epoch"].as_i64().unwrap(),
    });
    let client = client.with_authority(authority);
    let committed = git(&["cat-file", "-p", "HEAD:note.txt"]);
    assert_eq!(committed.as_bytes(), b"first commit\n");
    let objects = f
        .run(client.list_directory(Some(".git/objects"), None, 1000))
        .unwrap();
    assert!(!objects.entries.is_empty());
    let h = f
        .run(client.open_handle(
            &anchorfs_client::wire::OpenHandle {
                path: "note.txt".into(),
                read: true,
                write: false,
                create: false,
                exclusive: false,
                truncate: false,
                mode: None,
            },
            &anchorfs_client::retry::IdempotencyKey::new(),
        ))
        .unwrap();
    assert_eq!(
        &f.run(client.read_range(h.handle_id, 0, 100)).unwrap().0[..],
        b"first commit\n"
    );
    f.run(client.close_handle(h.handle_id, &anchorfs_client::retry::IdempotencyKey::new()))
        .unwrap();
    drop(m);
    f.invariants();
}
