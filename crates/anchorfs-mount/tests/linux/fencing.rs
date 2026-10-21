use super::{fixture::*, mountpoint::*};
use anchorfs_client::auth::WorkflowAuthority;
use anchorfs_core::Secret;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::FileExt,
    sync::Arc,
    time::{Duration, Instant},
};
fn stale(error: std::io::Error) {
    assert_eq!(error.raw_os_error(), Some(libc::ESTALE));
}
fn lease(f: &Fixture) -> Value {
    f.raw(
        &f.host,
        Some(&f.host_session),
        "POST",
        "/workflow-lease",
        Some(json!({})),
    )
}
fn bound(f: &Fixture, l: &Value) -> Value {
    f.raw(
        &f.host,
        None,
        "POST",
        "/sessions",
        Some(json!({"credential_id":f.worker.id,"bind_to_workflow_lease":l["token"]})),
    )
}
fn proof(l: &Value) -> (&str, i64) {
    (
        l["token"].as_str().unwrap(),
        l["mutation_epoch"].as_i64().unwrap(),
    )
}
fn proxy(f: &mut Fixture) -> super::relay::ApiProxy {
    let address = f.endpoint.strip_prefix("http://").unwrap().parse().unwrap();
    let proxy = f.run(super::relay::ApiProxy::start(address));
    f.endpoint = proxy.endpoint.clone();
    proxy
}
#[test]
fn paused_mount_past_expiry_is_fenced() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("f", b"old");
    let m = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(m.path.join("f"))
        .unwrap();
    let before = f.count("write_at");
    f.sql(
        "UPDATE handles SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1",
    );
    stale(file.write_at(b"bad", 0).unwrap_err());
    let mut bytes = [0; 3];
    stale(file.read_at(&mut bytes, 0).unwrap_err());
    let another = fs::File::open(m.path.join("f")).unwrap();
    drop(another);
    f.sql("UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=(SELECT client_session_id FROM operations WHERE workspace_id=$1 AND op_type='handle_open' AND credential_id=(SELECT id FROM credentials WHERE kind='worker' AND workspace_id=$1) LIMIT 1)");
    stale(fs::File::open(m.path.join("f")).unwrap_err());
    assert_eq!(f.count("write_at"), before);
    close_ok(file);
    drop(m);
    f.invariants();
}
#[test]
fn lease_bound_mount_publishes_and_ordinary_mount_is_busy() {
    let _serial = serial();
    let f = Fixture::new();
    f.put("existing", b"readable");
    let l = lease(&f);
    let s = bound(&f, &l);
    let (token, epoch) = proof(&l);
    let a = TempMount::process(&f, &f.worker, Some(&s), Some((token, epoch)), 1048576);
    let b = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    fs::write(a.path.join("bound"), b"lease bytes").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(a.path.join("bound"))
        .unwrap();
    assert_eq!(
        fs::write(b.path.join("blocked"), b"bad")
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EBUSY)
    );
    assert_eq!(fs::read(b.path.join("existing")).unwrap(), b"readable");
    let epochs:Vec<i64>=f.run(sqlx::query_scalar("SELECT DISTINCT mutation_epoch_at_start FROM operations WHERE workspace_id=$1 AND op_type='write_at' AND client_session_id=$2").bind(f.workspace).bind(s["session_id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap()).fetch_all(&f.pool)).unwrap();
    assert_eq!(epochs, vec![epoch]);
    lease_call(&f, "DELETE", "/workflow-lease", token, epoch);
    stale(file.write_at(b"bad", 0).unwrap_err());
    let mut read = [0; 3];
    stale(file.read_at(&mut read, 0).unwrap_err());
    drop(file);
    let rebound = f.raw(
        &f.host,
        Some(&f.host_session),
        "POST",
        &format!("/sessions/{}/rebind", s["session_id"].as_str().unwrap()),
        Some(json!({"workflow_lease_token":null})),
    );
    let successor = TempMount::process(&f, &f.worker, Some(&rebound), None, 1048576);
    fs::write(successor.path.join("successor"), b"yes").unwrap();
    fs::write(b.path.join("ordinary"), b"yes").unwrap();
    drop(successor);
    drop(a);
    drop(b);
    f.invariants();
}
#[test]
fn takeover_fences_bound_mount() {
    let _serial = serial();
    let f = Fixture::new();
    let l = lease(&f);
    let s = bound(&f, &l);
    let a = TempMount::process(&f, &f.worker, Some(&s), Some(proof(&l)), 1048576);
    fs::write(a.path.join("before"), b"old").unwrap();
    f.sql("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1");
    let successor_host=Credential::from(&f.run(raw(&f.endpoint,&f.admin,None,"POST","/v1/credentials",Some(json!({"workspace_id":f.workspace,"kind":"host","role":"write","uid":unsafe{libc::geteuid()},"gid":unsafe{libc::getegid()},"max_reservation_bytes":1u64<<30})))));
    assert_ne!(successor_host.id, f.host.id);
    let host_session = f.raw(
        &successor_host,
        None,
        "POST",
        "/sessions",
        Some(json!({"credential_id":successor_host.id})),
    );
    let next = f.raw(
        &successor_host,
        Some(&host_session),
        "POST",
        "/workflow-lease/takeover",
        Some(json!({})),
    );
    let session = f.raw(
        &successor_host,
        None,
        "POST",
        "/sessions",
        Some(json!({"credential_id":f.worker.id,"bind_to_workflow_lease":next["token"]})),
    );
    let b = TempMount::process(&f, &f.worker, Some(&session), Some(proof(&next)), 1048576);
    stale(fs::write(a.path.join("stale"), b"bad").unwrap_err());
    fs::write(b.path.join("after"), b"new").unwrap();
    assert_eq!(f.read("before"), b"old");
    assert_eq!(f.read("after"), b"new");
    assert_eq!(f.count("write_at"), 2);
    drop(a);
    drop(b);
    f.invariants();
}
#[test]
fn expired_lease_latches_bound_mount_without_takeover() {
    let _serial = serial();
    let mut f = Fixture::new();
    let l = lease(&f);
    let s = bound(&f, &l);
    let relay = proxy(&mut f);
    let a = TempMount::process(&f, &f.worker, Some(&s), Some(proof(&l)), 1048576);
    fs::write(a.path.join("f"), b"old").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(a.path.join("f"))
        .unwrap();
    f.sql("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1");
    stale(file.write_at(b"bad", 0).unwrap_err());
    let before = relay.count();
    let mut bytes = [0; 3];
    stale(file.read_at(&mut bytes, 0).unwrap_err());
    stale(fs::metadata(&a.path).unwrap_err());
    assert_eq!(relay.count(), before);
    let direct = native(&f.endpoint, f.workspace, &f.worker, Some(&s));
    direct.authority.write().unwrap().workflow = Some(WorkflowAuthority {
        token: Secret::new(proof(&l).0.into()),
        epoch: proof(&l).1,
    });
    f.run(direct.stat(None)).unwrap();
    assert_eq!(f.count("write_at"), 1);
    drop(file);
    drop(a);
    f.invariants();
}
#[test]
fn revocation_fences_mount() {
    let _serial = serial();
    for credential in [false, true] {
        let mut f = Fixture::new();
        f.put("f", b"old");
        let relay = proxy(&mut f);
        let a = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(a.path.join("f"))
            .unwrap();
        let before = f.count("write_at");
        if credential {
            f.run(raw(
                &f.endpoint,
                &f.host,
                Some(&f.host_session),
                "DELETE",
                &format!("/v1/credentials/{}", f.worker.id),
                None,
            ));
        } else {
            f.raw(
                &f.host,
                Some(&f.host_session),
                "DELETE",
                &format!("/sessions/{}", f.session["session_id"].as_str().unwrap()),
                None,
            );
        }
        stale(file.write_at(b"bad", 0).unwrap_err());
        let count = relay.count();
        stale(fs::metadata(&a.path).unwrap_err());
        assert_eq!(relay.count(), count);
        assert_eq!(f.count("write_at"), before);
        close_ok(file);
        f.put("host", b"continues");
        let successor = TempMount::process(&f, &f.host, None, None, 1048576);
        assert_eq!(fs::read(successor.path.join("host")).unwrap(), b"continues");
        drop(successor);
        drop(a);
        f.invariants();
    }
}
#[test]
fn renewal_keeps_handles_and_session_alive() {
    let _serial = serial();
    let f = Fixture::with(
        |c| {
            c.handle_ttl = Duration::from_secs(20);
            c.client_session_ttl = Duration::from_secs(40);
        },
        1 << 30,
        Arc::new(anchorfs_service::pipeline::faults::NoFaults),
    );
    f.put("f", b"old");
    let m = TempMount::in_process(&f);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(m.path.join("f"))
        .unwrap();
    let before: time::OffsetDateTime = f
        .run(
            sqlx::query_scalar("SELECT max(expires_at) FROM handles WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    let session_before: time::OffsetDateTime = f
        .run(
            sqlx::query_scalar("SELECT max(expires_at) FROM client_sessions WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    std::thread::sleep(Duration::from_secs(65));
    let mut bytes = [0; 3];
    file.read_exact_at(&mut bytes, 0).unwrap();
    assert_eq!(&bytes, b"old");
    file.write_all_at(b"new", 0).unwrap();
    let after: time::OffsetDateTime = f
        .run(
            sqlx::query_scalar("SELECT max(expires_at) FROM handles WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    assert!(after > before);
    let session_after: time::OffsetDateTime = f
        .run(
            sqlx::query_scalar("SELECT max(expires_at) FROM client_sessions WHERE workspace_id=$1")
                .bind(f.workspace)
                .fetch_one(&f.pool),
        )
        .unwrap();
    assert!(session_after > session_before);
    assert!(f.count("handle_renew") >= 5);
    assert!(f.count("session_renew") >= 3);
    drop(file);
    drop(m);
    f.invariants();
}
#[test]
fn sigstop_paused_mount_is_fenced_on_resume() {
    let _serial = serial();
    let f = Fixture::with(
        |c| {
            c.handle_ttl = Duration::from_secs(20);
            c.client_session_ttl = Duration::from_secs(40);
        },
        1 << 30,
        Arc::new(anchorfs_service::pipeline::faults::NoFaults),
    );
    f.put("f", b"old");
    let mut m = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(m.path.join("f"))
        .unwrap();
    let before = f.count("write_at");
    m.signal("-STOP");
    std::thread::sleep(Duration::from_secs(45));
    m.signal("-CONT");
    let mut bytes = [0; 3];
    stale(file.read_at(&mut bytes, 0).unwrap_err());
    stale(file.write_at(b"bad", 0).unwrap_err());
    stale(fs::File::open(m.path.join("f")).unwrap_err());
    assert_eq!(f.count("write_at"), before);
    drop(file);
    m.stop();
    f.invariants();
}
struct Gate {
    armed: std::sync::atomic::AtomicBool,
    arrived: std::sync::atomic::AtomicBool,
    release: tokio::sync::watch::Sender<bool>,
}
impl anchorfs_service::pipeline::faults::FaultInjector for Gate {
    fn at(
        &self,
        point: anchorfs_service::pipeline::faults::FaultPoint,
        _: Option<anchorfs_core::OperationId>,
    ) -> anchorfs_service::pipeline::faults::FaultAction {
        use std::sync::atomic::Ordering::SeqCst;
        if point == anchorfs_service::pipeline::faults::FaultPoint::BeforePublish
            && self.armed.swap(false, SeqCst)
        {
            self.arrived.store(true, SeqCst);
            anchorfs_service::pipeline::faults::FaultAction::Wait(self.release.subscribe())
        } else {
            anchorfs_service::pipeline::faults::FaultAction::Continue
        }
    }
}
#[test]
fn ordinary_mount_survives_lease_race() {
    let _serial = serial();
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let (release, _) = tokio::sync::watch::channel(false);
    let gate = Arc::new(Gate {
        armed: AtomicBool::new(false),
        arrived: AtomicBool::new(false),
        release,
    });
    let f = Fixture::with(|_| {}, 1 << 30, gate.clone());
    f.put("f", b"old");
    let m = TempMount::process(&f, &f.worker, Some(&f.session), None, 1048576);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(m.path.join("f"))
        .unwrap();
    let before = f.count("write_at");
    gate.armed.store(true, SeqCst);
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| file.write_at(b"stale", 0));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gate.arrived.load(SeqCst) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let l = lease(&f);
        lease_call(&f, "DELETE", "/workflow-lease", proof(&l).0, proof(&l).1);
        gate.release.send(true).unwrap();
        stale(writer.join().unwrap().unwrap_err());
    });
    file.write_all_at(b"new", 0).unwrap();
    assert_eq!(f.count("write_at"), before + 1);
    let failures:i64=f.run(sqlx::query_scalar("SELECT count(*) FROM operations WHERE workspace_id=$1 AND op_type='write_at' AND state='failed'").bind(f.workspace).fetch_one(&f.pool)).unwrap();
    assert_eq!(failures, 1);
    drop(file);
    drop(m);
    f.invariants();
}

fn close_ok(file: std::fs::File) {
    use std::os::fd::IntoRawFd;
    assert_eq!(unsafe { libc::close(file.into_raw_fd()) }, 0);
}
