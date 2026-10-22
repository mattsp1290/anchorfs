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
        .session(&f.host_session)
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
    f.put("api-file", b"from API over verified TLS");
    on_mount(|| {
        assert_eq!(
            fs::read(m.path.join("api-file")).unwrap(),
            b"from API over verified TLS"
        )
    });
    m.stop();
    f.invariants();
}

#[test]
fn https_mount_without_trusted_ca_exits_before_mounting() {
    let _serial = serial();
    let f = Fixture::new_tls();
    f.run(f.api.stat(None)).unwrap();
    let before = effect_counts(&f);
    let refused = TempMount::builder(&f).fails_with(4);
    assert_log_redacted(&refused, &f);
    drop(refused);
    assert_eq!(effect_counts(&f), before);
    f.run(f.api.stat(None)).unwrap();
    let other_ca = &f.pki.as_ref().unwrap().files.other_ca;
    let refused = TempMount::builder(&f).ca_bundle(other_ca).fails_with(4);
    assert_log_redacted(&refused, &f);
    drop(refused);
    assert_eq!(effect_counts(&f), before);
    f.run(f.api.stat(None)).unwrap();
    let mut trusted = TempMount::builder(&f)
        .ca_bundle(f.ca_bundle.as_ref().unwrap())
        .process();
    on_mount(|| assert!(fs::metadata(&trusted.path).unwrap().is_dir()));
    trusted.stop();
    f.invariants();
}

fn assert_log_redacted(mount: &TempMount, f: &Fixture) {
    let log = fs::read(mount.path.with_extension("log")).unwrap();
    assert!(!log.windows(b"-----BEGIN".len()).any(|w| w == b"-----BEGIN"));
    for material in [
        &f.pki.as_ref().unwrap().pki.cert_pem,
        &f.pki.as_ref().unwrap().pki.key_pem,
    ] {
        assert!(!log.windows(material.len()).any(|w| w == material));
    }
}

fn effect_counts(f: &Fixture) -> (i64, i64, i64) {
    let admin: i64 = f
        .run(sqlx::query_scalar("SELECT count(*) FROM admin_operations").fetch_one(&f.pool))
        .unwrap();
    (f.session_count(), f.receipt_count(), admin)
}
