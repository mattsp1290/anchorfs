use super::{fixture::*, mountpoint::*};
use anchorfs_client::retry::IdempotencyKey;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt},
    },
};
#[test]
fn mount_lookup_stat_readdir_match_api() {
    let _serial = serial();
    let f = Fixture::with(
        |c| c.max_list_page = 2,
        1 << 30,
        std::sync::Arc::new(anchorfs_service::pipeline::faults::NoFaults),
    );
    f.run(f.api.create_directory("a", 0o755, &IdempotencyKey::new()))
        .unwrap();
    f.put("a/b.txt", b"hello");
    f.run(f.api.create_symlink("a/l", "b.txt", &IdempotencyKey::new()))
        .unwrap();
    for n in ["c", "d", "e"] {
        f.put(&format!("a/{n}"), b"x");
    }
    let m = TempMount::in_process(&f);
    on_mount(|| {
        let expected = f.run(f.api.stat(Some("a/b.txt"))).unwrap();
        let observed = fs::symlink_metadata(m.path.join("a/b.txt")).unwrap();
        assert_eq!(observed.ino(), expected.inode_id as u64);
        assert_eq!(observed.size(), expected.size);
        assert_eq!(observed.mode() & 0o777, expected.mode);
        assert_eq!(observed.uid(), expected.uid);
        assert_eq!(observed.gid(), expected.gid);
        assert_eq!(observed.nlink(), expected.nlink as u64);
        assert_eq!(observed.mtime(), expected.mtime.unix_timestamp());
        assert_eq!(fs::metadata(&m.path).unwrap().ino(), 1);
        assert_eq!(
            fs::read_link(m.path.join("a/l"))
                .unwrap()
                .as_os_str()
                .as_bytes(),
            b"b.txt"
        );
        let mut api = Vec::new();
        let mut cursor = None;
        loop {
            let page = f
                .run(f.api.list_directory(Some("a"), cursor.as_deref(), 2))
                .unwrap();
            api.extend(page.entries.into_iter().map(|e| e.name));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let mut names = fs::read_dir(m.path.join("a"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        api.sort();
        assert_eq!(names, api);
    });
    drop(m);
    f.invariants();
}
#[test]
fn mount_write_fsync_release_visible_to_api_and_back() {
    let _serial = serial();
    let f = Fixture::new();
    let m = TempMount::process(&f, &f.worker, Some(&f.session), None, 262144);
    on_mount(|| {
        let data = vec![71; 1048577];
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(m.path.join("f"))
            .unwrap();
        assert_eq!(file.write(&data).unwrap(), data.len());
        file.sync_all().unwrap();
        assert_eq!(f.read("f"), data);
        let h = f.open("f", false);
        let (observed, generation) = f
            .run(f.api.read_range(h.handle_id, 0, data.len() as u32))
            .unwrap();
        assert_eq!(&observed[..], &data);
        let committed_generation:String=f.run(sqlx::query_scalar("SELECT result->>'generation' FROM operations WHERE workspace_id=$1 AND op_type='write_at' AND state='committed' ORDER BY committed_at DESC LIMIT 1").bind(f.workspace).fetch_one(&f.pool)).unwrap();
        assert_eq!(generation, committed_generation.parse::<i64>().unwrap());
        f.run(f.api.close_handle(h.handle_id, &IdempotencyKey::new()))
            .unwrap();
        assert!(f.count("write_at") >= 5);
        f.put("f", b"new");
        let mut read = [0u8; 3];
        assert_eq!(file.read_at(&mut read, 0).unwrap(), 3);
        assert_eq!(&read, b"new");
    });
    drop(m);
    f.invariants();
}
#[test]
fn two_mount_processes_share_committed_files() {
    let _serial = serial();
    let f = Fixture::new();
    let second = f.raw(
        &f.host,
        None,
        "POST",
        "/sessions",
        Some(serde_json::json!({"credential_id":f.worker.id})),
    );
    let mut a = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let b = TempMount::process(&f, &f.worker, Some(&second), None, 1048576);
    on_mount(|| {
        fs::create_dir(a.path.join("d")).unwrap();
        fs::write(a.path.join("d/f"), b"durable").unwrap();
        assert_eq!(fs::read(b.path.join("d/f")).unwrap(), b"durable");
        assert_eq!(fs::metadata(b.path.join("d/f")).unwrap().len(), 7);
        assert_eq!(fs::read_dir(b.path.join("d")).unwrap().count(), 1);
    });
    a.stop();
    assert_eq!(fs::read(b.path.join("d/f")).unwrap(), b"durable");
    f.raw(
        &f.host,
        Some(&f.host_session),
        "DELETE",
        &format!("/sessions/{}", f.session["session_id"].as_str().unwrap()),
        None,
    );
    assert_eq!(fs::read(b.path.join("d/f")).unwrap(), b"durable");
    drop(b);
    f.invariants();
}
#[test]
fn reader_sees_remote_write_on_next_read() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("f", b"old");
    let m = TempMount::in_process(&f);
    on_mount(|| {
        let mut file = fs::File::open(m.path.join("f")).unwrap();
        let mut data = [0u8; 3];
        file.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"old");
        f.put("f", b"new version");
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"new");
        assert_eq!(fs::metadata(m.path.join("f")).unwrap().len(), 11);
    });
    drop(m);
    f.invariants();
}
