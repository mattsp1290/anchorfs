use super::{fixture::*, mountpoint::*};
use std::{
    fs::{self, OpenOptions},
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
};
fn errno(result: i32, expected: i32) {
    assert_eq!(result, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(expected)
    );
}
#[test]
fn unsupported_operations_are_explicit() {
    let _serial = serial();
    let mut f = Fixture::new();
    let address = f.endpoint.strip_prefix("http://").unwrap().parse().unwrap();
    let relay = f.run(super::relay::ApiProxy::start(address));
    f.endpoint = relay.endpoint.clone();
    f.put("f", b"mapped bytes");
    let m = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(m.path.join("f"))
        .unwrap();
    let before: i64 = f
        .run(
            sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    let mutations = || {
        relay
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.to_ascii_lowercase().contains("idempotency-key:"))
            .count()
    };
    let requests = mutations();
    on_mount(|| unsafe {
        errno(
            libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB),
            libc::ENOLCK,
        );
        let mut lock: libc::flock = std::mem::zeroed();
        lock.l_type = libc::F_WRLCK as i16;
        lock.l_whence = libc::SEEK_SET as i16;
        errno(
            libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock),
            libc::ENOLCK,
        );
        errno(
            libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock),
            libc::ENOLCK,
        );
        let fifo =
            std::ffi::CString::new(m.path.join("fifo").as_os_str().as_encoded_bytes()).unwrap();
        errno(libc::mkfifo(fifo.as_ptr(), 0o600), libc::EPERM);
        let device =
            std::ffi::CString::new(m.path.join("device").as_os_str().as_encoded_bytes()).unwrap();
        errno(
            libc::mknod(device.as_ptr(), libc::S_IFCHR | 0o600, 0),
            libc::EPERM,
        );
        let path = std::ffi::CString::new(m.path.join("f").as_os_str().as_encoded_bytes()).unwrap();
        let key = c"user.anchorfs-test";
        errno(
            libc::setxattr(path.as_ptr(), key.as_ptr(), b"x".as_ptr().cast(), 1, 0),
            libc::EOPNOTSUPP,
        );
        let dest =
            std::ffi::CString::new(m.path.join("renamed").as_os_str().as_encoded_bytes()).unwrap();
        errno(
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                path.as_ptr(),
                libc::AT_FDCWD,
                dest.as_ptr(),
                libc::RENAME_NOREPLACE,
            ) as i32,
            libc::EINVAL,
        );
        assert_eq!(
            fs::set_permissions(m.path.join("f"), fs::Permissions::from_mode(0o4755))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EPERM)
        );
        let shared = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        );
        if shared != libc::MAP_FAILED {
            libc::munmap(shared, 4096);
            panic!("shared writable mmap unexpectedly succeeded");
        }
        eprintln!("shared mmap errno: {}", std::io::Error::last_os_error());
        let private = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        );
        eprintln!(
            "read-only private mmap succeeded: {}",
            private != libc::MAP_FAILED
        );
        if private != libc::MAP_FAILED {
            libc::munmap(private, 4096);
        }
    });
    assert_eq!(mutations(), requests);
    let after: i64 = f
        .run(
            sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    assert_eq!(after, before);
    drop(file);
    drop(m);
    f.invariants();
}
