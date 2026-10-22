use super::{fixture::Fixture, mountpoint::*};
use std::{
    fs::{self, File},
    io::Write,
};

#[test]
fn https_mount_with_ca_bundle_round_trips() {
    let _serial = serial();
    let f = Fixture::new_tls();
    let mut m = TempMount::builder(&f)
        .ca_bundle(f.ca_bundle.as_ref().unwrap())
        .process();
    on_mount(|| {
        let path = m.path.join("tls-file");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"mounted over verified TLS").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"mounted over verified TLS");
        assert_eq!(fs::metadata(path).unwrap().len(), 25);
    });
    assert_eq!(f.read("tls-file"), b"mounted over verified TLS");
    m.stop();
    f.invariants();
}

#[test]
fn https_mount_without_trusted_ca_exits_before_mounting() {
    let _serial = serial();
    let f = Fixture::new_tls();
    f.run(f.api.stat(None)).unwrap();
    let before = f.session_count();
    drop(TempMount::builder(&f).fails_with(4));
    assert_eq!(f.session_count(), before);
    f.run(f.api.stat(None)).unwrap();
    let other_ca = &f.pki.as_ref().unwrap().files.other_ca;
    drop(TempMount::builder(&f).ca_bundle(other_ca).fails_with(4));
    assert_eq!(f.session_count(), before);
    f.run(f.api.stat(None)).unwrap();
    f.invariants();
}
