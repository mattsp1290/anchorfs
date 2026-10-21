use super::{fixture::*, mountpoint::*};
use std::{
    fs,
    io::{BufRead, Write},
    os::{fd::AsRawFd, unix::fs::FileExt},
    process::{Command, Stdio},
    time::Duration,
};
#[test]
fn open_file_helper() {
    let Ok(path) = std::env::var("ANCHORFS_TEST_HELD_FILE") else {
        return;
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    println!("ANCHORFS_HELPER_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    let mut bytes = [0; 3];
    file.read_exact_at(&mut bytes, 0).unwrap();
    assert_eq!(&bytes, b"old");
    file.write_all_at(b"held", 0).unwrap();
}
#[test]
fn sigterm_with_open_files_keeps_serving() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("f", b"old");
    let mut m = TempMount::process(&f, &f.host, None, None, 1048576);
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lifecycle::open_file_helper", "--nocapture"])
        .env("ANCHORFS_TEST_HELD_FILE", m.path.join("f"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = std::io::BufReader::new(helper.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line.contains("ANCHORFS_HELPER_READY") {
            break;
        }
    }
    let closed = f.count("handle_close");
    m.signal("-TERM");
    std::thread::sleep(Duration::from_millis(500));
    assert!(m.process.as_mut().unwrap().try_wait().unwrap().is_none());
    assert_eq!(f.count("handle_close"), closed);
    helper
        .stdin
        .take()
        .unwrap()
        .write_all(b"continue\n")
        .unwrap();
    assert!(helper.wait().unwrap().success());
    assert_eq!(f.read("f"), b"held");
    m.stop();
    assert_eq!(f.count("session_revoke"), 1);
    f.invariants();
}
#[test]
fn replaced_ancestor_directory_is_estale() {
    let _serial = serial();
    let f = Fixture::new();
    f.run(
        f.api
            .create_directory("d", 0o755, &anchorfs_client::retry::IdempotencyKey::new()),
    )
    .unwrap();
    f.put("d/x", b"x");
    let a = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let b = TempMount::in_process(&f);
    let dir = fs::File::open(b.path.join("d")).unwrap();
    fs::rename(a.path.join("d"), a.path.join("old")).unwrap();
    fs::create_dir(a.path.join("d")).unwrap();
    let before = f.count("handle_open");
    let unlink_before = f.count("unlink");
    let mkdir_before = f.count("directory_create");
    unsafe {
        let fd = libc::openat(
            dir.as_raw_fd(),
            c"y".as_ptr(),
            libc::O_CREAT | libc::O_WRONLY,
            0o600,
        );
        assert_eq!(fd, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESTALE)
        );
        assert_eq!(libc::unlinkat(dir.as_raw_fd(), c"x".as_ptr(), 0), -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESTALE)
        );
        assert_eq!(libc::mkdirat(dir.as_raw_fd(), c"sub".as_ptr(), 0o700), -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESTALE)
        );
    }
    assert_eq!(f.count("handle_open"), before);
    assert_eq!(f.count("unlink"), unlink_before);
    assert_eq!(f.count("directory_create"), mkdir_before);
    assert_eq!(fs::read_dir(a.path.join("d")).unwrap().count(), 0);
    drop(dir);
    drop(a);
    drop(b);
    f.invariants();
}
