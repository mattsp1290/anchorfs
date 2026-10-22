//! Synthetic per-test PKI. Never uses a host trust key or committed PEM fixture.
#![allow(dead_code)]
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, SanType};
use std::{
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

pub struct TestPki {
    pub ca_pem: Vec<u8>,
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub other_ca_pem: Vec<u8>,
}
fn params() -> CertificateParams {
    let mut p = CertificateParams::default();
    let now = time::OffsetDateTime::now_utc();
    p.not_before = now - time::Duration::hours(1);
    p.not_after = now + time::Duration::days(1);
    p
}
fn ca() -> (Vec<u8>, Issuer<'static, KeyPair>) {
    let mut p = params();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let key = KeyPair::generate().unwrap();
    let cert = p.self_signed(&key).unwrap();
    (cert.pem().into_bytes(), Issuer::new(p, key))
}
pub fn generate() -> TestPki {
    let (ca_pem, issuer) = ca();
    let mut p = params();
    p.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().unwrap()),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];
    let key = KeyPair::generate().unwrap();
    let cert = p.signed_by(&key, &issuer).unwrap();
    TestPki {
        ca_pem,
        cert_pem: cert.pem().into_bytes(),
        key_pem: key.serialize_pem().into_bytes(),
        other_ca_pem: ca().0,
    }
}
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub ca: PathBuf,
    pub other_ca: PathBuf,
}
pub fn write(dir: &Path, pki: &TestPki) -> TlsFiles {
    let files = TlsFiles {
        cert: dir.join("cert.pem"),
        key: dir.join("key.pem"),
        ca: dir.join("ca.pem"),
        other_ca: dir.join("other-ca.pem"),
    };
    for (path, bytes) in [
        (&files.cert, &pki.cert_pem),
        (&files.ca, &pki.ca_pem),
        (&files.other_ca, &pki.other_ca_pem),
    ] {
        std::fs::write(path, bytes).unwrap();
    }
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&files.key)
        .unwrap()
        .write_all(&pki.key_pem)
        .unwrap();
    files
}
pub struct TempPki {
    pub pki: TestPki,
    pub files: TlsFiles,
    dir: PathBuf,
}
impl TempPki {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("anchorfs-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let pki = generate();
        let files = write(&dir, &pki);
        Self { pki, files, dir }
    }
}
impl Drop for TempPki {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
