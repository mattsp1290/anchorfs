use super::{fixture::*, mountpoint::*};
use anchorfs_client::retry::IdempotencyKey;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{FileExt, MetadataExt, PermissionsExt},
};

#[test]
fn rename_over_open_and_unlink_open_keep_handle() {
    let _serial = serial();
    let f = Fixture::new();
    let m = TempMount::in_process(&f);
    on_mount(|| {
        fs::write(m.path.join("f1"), b"old").unwrap();
        fs::hard_link(m.path.join("f1"), m.path.join("kept")).unwrap();
        fs::write(m.path.join("f2"), b"new").unwrap();
        let old = OpenOptions::new()
            .read(true)
            .write(true)
            .open(m.path.join("f1"))
            .unwrap();
        let old_ino = old.metadata().unwrap().ino();
        fs::rename(m.path.join("f2"), m.path.join("f1")).unwrap();
        let mut bytes = [0u8; 3];
        old.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes, b"old");
        old.write_all_at(b"old extended", 0).unwrap();
        assert_eq!(old.metadata().unwrap().ino(), old_ino);
        assert_eq!(old.metadata().unwrap().len(), 12);
        assert_eq!(f.read("kept"), b"old extended");
        old.set_len(2).unwrap();
        assert_eq!(f.read("kept"), b"ol");
        assert_eq!(fs::read(m.path.join("f1")).unwrap(), b"new");
        fs::write(m.path.join("f3"), b"orphan").unwrap();
        let orphan = OpenOptions::new()
            .read(true)
            .write(true)
            .open(m.path.join("f3"))
            .unwrap();
        fs::remove_file(m.path.join("f3")).unwrap();
        orphan.write_all_at(b"live", 0).unwrap();
        let mut data = [0; 4];
        orphan.read_exact_at(&mut data, 0).unwrap();
        assert_eq!(&data, b"live");
        assert_eq!(orphan.metadata().unwrap().len(), 6);
        drop(orphan);
        assert!(!m.path.join("f3").exists());
    });
    drop(m);
    f.put("g", b"old");
    f.put("h", b"new");
    let a = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let session = f.raw(
        &f.host,
        None,
        "POST",
        "/sessions",
        Some(serde_json::json!({"credential_id":f.worker.id})),
    );
    let b = TempMount::process(&f, &f.worker, Some(&session), None, 1048576);
    let old = fs::File::open(b.path.join("g")).unwrap();
    let ino = old.metadata().unwrap().ino();
    fs::rename(a.path.join("h"), a.path.join("g")).unwrap();
    assert_eq!(old.metadata().unwrap().ino(), ino);
    let mut bytes = [0; 3];
    old.read_exact_at(&mut bytes, 0).unwrap();
    assert_eq!(&bytes, b"old");
    assert_eq!(fs::read(b.path.join("g")).unwrap(), b"new");
    drop(old);
    drop(a);
    drop(b);
    f.invariants();
}
#[test]
fn hard_link_names_survive_unlink_of_one() {
    let _serial = serial();
    let f = Fixture::new();
    let m = TempMount::in_process(&f);
    on_mount(|| {
        fs::write(m.path.join("a"), b"linked").unwrap();
        fs::hard_link(m.path.join("a"), m.path.join("b")).unwrap();
        let fd = fs::File::open(m.path.join("a")).unwrap();
        let ino = fd.metadata().unwrap().ino();
        fs::remove_file(m.path.join("a")).unwrap();
        assert_eq!(fs::metadata(m.path.join("b")).unwrap().ino(), ino);
        assert_eq!(fs::read(m.path.join("b")).unwrap(), b"linked");
        fs::remove_file(m.path.join("b")).unwrap();
        let mut data = [0; 6];
        fd.read_exact_at(&mut data, 0).unwrap();
        assert_eq!(&data, b"linked");
        assert_eq!(fd.metadata().unwrap().nlink(), 0);
        assert!(!m.path.join("b").exists());
    });
    drop(m);
    f.invariants();
}
#[test]
fn append_handles_use_server_eof() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("f", b"seed");
    let second = f.raw(
        &f.host,
        None,
        "POST",
        "/sessions",
        Some(serde_json::json!({"credential_id":f.worker.id})),
    );
    let a = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let b = TempMount::process(&f, &f.worker, Some(&second), None, 1048576);
    let before = f.count("append");
    on_mount(|| {
        let mut fa = OpenOptions::new()
            .append(true)
            .open(a.path.join("f"))
            .unwrap();
        let mut fb = OpenOptions::new()
            .append(true)
            .open(b.path.join("f"))
            .unwrap();
        for i in 0..64u8 {
            fa.write_all(&vec![i; 1024]).unwrap();
            fb.write_all(&vec![i + 64; 1024]).unwrap();
        }
    });
    let data = f.read("f");
    assert_eq!(&data[..4], b"seed");
    let mut records = data[4..]
        .chunks_exact(1024)
        .map(|c| {
            assert!(c.iter().all(|b| *b == c[0]));
            c[0]
        })
        .collect::<Vec<_>>();
    records.sort();
    assert_eq!(records, (0..128u8).collect::<Vec<_>>());
    assert_eq!(f.count("append") - before, 128);
    drop(a);
    drop(b);
    f.invariants();
}
#[test]
fn partial_write_reports_committed_bytes() {
    let _serial = serial();
    let f = Fixture::with(
        |_| {},
        100 * 1024,
        std::sync::Arc::new(anchorfs_service::pipeline::faults::NoFaults),
    );
    let m = TempMount::process(&f, &f.worker, Some(&f.session), None, 65536);
    on_mount(|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(m.path.join("f"))
            .unwrap();
        assert_eq!(file.write(&vec![9; 3 * 65536]).unwrap(), 65536);
        assert_eq!(
            file.write_at(&vec![7; 65536], 65536)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EDQUOT)
        );
    });
    assert_eq!(f.read("f").len(), 65536);
    drop(m);
    f.invariants();
}
#[test]
fn setattr_times_mode_truncate() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("f", b"abcdef");
    let m = TempMount::in_process(&f);
    on_mount(|| {
        let path = std::ffi::CString::new(m.path.join("f").as_os_str().as_encoded_bytes()).unwrap();
        let times = [
            libc::timespec {
                tv_sec: 1700000000,
                tv_nsec: 123456000,
            },
            libc::timespec {
                tv_sec: 1700000001,
                tv_nsec: 234567000,
            },
        ];
        assert_eq!(
            unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), 0) },
            0
        );
        let invalid = [libc::timespec {
            tv_sec: 253_402_300_800,
            tv_nsec: 0,
        }; 2];
        assert_eq!(
            unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), invalid.as_ptr(), 0) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EINVAL)
        );
        let meta = fs::metadata(m.path.join("f")).unwrap();
        assert_eq!(meta.atime(), 1700000000);
        assert_eq!(meta.mtime_nsec(), 234567000);
        assert_eq!(
            unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), std::ptr::null(), 0) },
            0
        );
        assert!(fs::metadata(m.path.join("f")).unwrap().mtime() > 1700000001);
        fs::set_permissions(m.path.join("f"), fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            fs::metadata(m.path.join("f")).unwrap().mode() & 0o777,
            0o640
        );
        let changes = f.count("attributes_set");
        assert_eq!(
            unsafe { libc::chown(path.as_ptr(), unsafe_uid() + 1, unsafe_gid()) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
        assert_eq!(
            unsafe { libc::chown(path.as_ptr(), unsafe_uid(), unsafe_gid()) },
            0
        );
        assert_eq!(f.count("attributes_set"), changes);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(m.path.join("f"))
            .unwrap();
        file.set_len(2).unwrap();
        file.set_len(8).unwrap();
        let mut bytes = [1; 8];
        file.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes, b"ab\0\0\0\0\0\0");
        drop(file);
        let fd = unsafe { libc::truncate(path.as_ptr(), 3) };
        assert_eq!(fd, 0);
        assert_eq!(fs::metadata(m.path.join("f")).unwrap().len(), 3);
    });
    let worker = super::fixture::native(&f.endpoint, f.workspace, &f.worker, Some(&f.session));
    let e = f
        .run(worker.set_attributes(
            &anchorfs_client::wire::SetAttributes {
                path: Some("f".into()),
                uid: Some(unsafe_uid() + 1),
                gid: None,
                mode: None,
                atime: None,
                mtime: None,
            },
            &IdempotencyKey::new(),
        ))
        .unwrap_err();
    assert_eq!(e.code(), Some("forbidden"));
    drop(m);
    f.invariants();
}
fn unsafe_uid() -> u32 {
    unsafe { libc::geteuid() }
}
fn unsafe_gid() -> u32 {
    unsafe { libc::getegid() }
}
