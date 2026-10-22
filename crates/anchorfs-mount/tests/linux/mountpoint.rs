use super::fixture::{Credential, Fixture};
use anchorfs_mount::{
    config::{MountArgs, MountConfig},
    runtime::Mount,
};
use clap::Parser;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static SERIAL: Mutex<()> = Mutex::new(());
pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}
pub fn require_fuse() {
    assert!(
        Path::new("/dev/fuse").exists()
            && Command::new("fusermount3")
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success()),
        "FUSE unavailable: /dev/fuse missing or fusermount3 not on PATH"
    );
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "mount tests require non-root"
    );
}
fn local(path: &Path) {
    let out = Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let fs = String::from_utf8(out.stdout).unwrap();
    assert!(
        !matches!(fs.trim(), "nfs" | "nfs4"),
        "mount tests require local disk"
    );
}
fn mounted(path: &Path) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .any(|l| {
            l.split_whitespace()
                .nth(4)
                .map(|s| {
                    s.replace("\\040", " ")
                        .replace("\\011", "\t")
                        .replace("\\134", "\\")
                })
                .as_deref()
                == path.to_str()
        })
}
pub struct TempMount {
    pub path: PathBuf,
    pub process: Option<Child>,
    session: Option<fuser::BackgroundSession>,
    mount: Option<Arc<Mount>>,
    runtime: Option<tokio::runtime::Runtime>,
    cancel: Option<std::sync::mpsc::Sender<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    expired: Arc<AtomicBool>,
}
pub struct MountBuilder<'a> {
    fixture: &'a Fixture,
    credential: &'a Credential,
    session: Option<&'a Value>,
    workflow: Option<(&'a str, i64)>,
    max_io: u32,
    watchdog: Duration,
    ca_bundle: Option<&'a Path>,
    expected_exit: Option<i32>,
}
impl<'a> MountBuilder<'a> {
    pub fn credential(mut self, credential: &'a Credential) -> Self {
        self.credential = credential;
        self
    }
    pub fn session(mut self, session: &'a Value) -> Self {
        self.session = Some(session);
        self
    }
    pub fn workflow(mut self, workflow: (&'a str, i64)) -> Self {
        self.workflow = Some(workflow);
        self
    }
    pub fn max_io(mut self, bytes: u32) -> Self {
        self.max_io = bytes;
        self
    }
    pub fn watchdog(mut self, duration: Duration) -> Self {
        self.watchdog = duration;
        self
    }
    pub fn ca_bundle(mut self, path: &'a Path) -> Self {
        self.ca_bundle = Some(path);
        self
    }
    pub fn fails_with(mut self, code: i32) -> TempMount {
        self.expected_exit = Some(code);
        TempMount::start(self, true)
    }
    pub fn process(self) -> TempMount {
        TempMount::start(self, true)
    }
    pub fn in_process(self) -> TempMount {
        TempMount::start(self, false)
    }
}
impl TempMount {
    pub fn builder(fixture: &Fixture) -> MountBuilder<'_> {
        MountBuilder {
            fixture,
            credential: &fixture.host,
            session: None,
            workflow: None,
            max_io: 1048576,
            watchdog: Duration::from_secs(120),
            ca_bundle: None,
            expected_exit: None,
        }
    }
    pub fn in_process(f: &Fixture) -> Self {
        Self::builder(f).in_process()
    }
    fn start(builder: MountBuilder<'_>, process: bool) -> Self {
        let MountBuilder {
            fixture: f,
            credential,
            session,
            workflow,
            max_io,
            watchdog,
            ca_bundle,
            expected_exit,
        } = builder;
        require_fuse();
        local(&std::env::temp_dir());
        local(Path::new(env!("CARGO_MANIFEST_DIR")));
        local(std::env::current_exe().unwrap().parent().unwrap());
        let path = std::env::temp_dir()
            .join("anchorfs-mount-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path).unwrap();
        let mut args = vec![
            "mount".to_owned(),
            path.to_str().unwrap().into(),
            "--endpoint".into(),
            f.endpoint.clone(),
            "--workspace".into(),
            f.workspace.to_string(),
            "--max-io-bytes".into(),
            max_io.to_string(),
            "--handle-ttl-hint-ms".into(),
            f.config.handle_ttl.as_millis().to_string(),
            "--session-ttl-hint-ms".into(),
            f.config.client_session_ttl.as_millis().to_string(),
        ];
        if f.endpoint.starts_with("https://") {
            if let Some(ca) = ca_bundle {
                args.extend(["--ca-bundle".into(), ca.display().to_string()]);
            }
        } else {
            args.push("--allow-plaintext".into());
        }
        if let Some(s) = session {
            args.extend([
                "--session-id".into(),
                s["session_id"].as_str().unwrap().into(),
                "--session-epoch".into(),
                s["session_epoch"].as_i64().unwrap().to_string(),
            ]);
        }
        if let Some((_, epoch)) = workflow {
            args.extend(["--workflow-epoch".into(), epoch.to_string()]);
        }
        let mut this = Self {
            path,
            process: None,
            session: None,
            mount: None,
            runtime: None,
            cancel: None,
            watchdog: None,
            expired: Arc::new(AtomicBool::new(false)),
        };
        if process {
            let log = std::fs::File::create(this.path.with_extension("log")).unwrap();
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_anchorfs-mount"));
            cmd.args(&args)
                .env("ANCHORFS_MOUNT_CREDENTIAL", &credential.bearer)
                .env_remove("ANCHORFS_MOUNT_WORKFLOW_TOKEN")
                .stdout(Stdio::null())
                .stderr(log);
            if let Some((token, _)) = workflow {
                cmd.env("ANCHORFS_MOUNT_WORKFLOW_TOKEN", token);
            }
            this.process = Some(cmd.spawn().unwrap());
        } else {
            #[derive(Parser)]
            struct Cli {
                #[command(flatten)]
                args: MountArgs,
            }
            let cfg: MountConfig = Cli::parse_from(args)
                .args
                .with_secrets(&credential.bearer, workflow.map(|w| w.0))
                .unwrap();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap();
            let mount = rt.block_on(Mount::start(cfg, rt.handle().clone())).unwrap();
            this.session = Some(anchorfs_mount::platform::spawn(mount.clone()).unwrap());
            this.mount = Some(mount);
            this.runtime = Some(rt);
        }
        let (cancel, receive) = std::sync::mpsc::channel();
        let path = this.path.clone();
        let pid = this.process.as_ref().map(Child::id);
        let expired = this.expired.clone();
        this.cancel = Some(cancel);
        this.watchdog = Some(std::thread::spawn(move || {
            if receive.recv_timeout(watchdog).is_err() {
                expired.store(true, Ordering::SeqCst);
                // A helper's exec can flush inherited FUSE descriptors. Kill
                // a paused process directly before attempting cleanup helpers.
                if let Some(pid) = pid {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                let _ = Command::new("fusermount3").arg("-uz").arg(&path).status();
            }
        }));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !mounted(&this.path) {
            if let Some(child) = &mut this.process
                && let Some(status) = child.try_wait().unwrap()
            {
                if let Some(code) = expected_exit {
                    assert_eq!(
                        status.code(),
                        Some(code),
                        "{}",
                        std::fs::read_to_string(this.path.with_extension("log"))
                            .unwrap_or_default()
                    );
                    assert!(!mounted(&this.path));
                    return this;
                }
                panic!(
                    "mount exited {status}: {}",
                    std::fs::read_to_string(this.path.with_extension("log")).unwrap_or_default()
                );
            }
            assert!(Instant::now() < deadline, "mount startup timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            expected_exit.is_none(),
            "expected mount exit {expected_exit:?}, but it mounted"
        );
        this
    }
    pub fn signal(&self, signal: nix::sys::signal::Signal) {
        use nix::{sys::signal::kill, unistd::Pid};
        // Avoid forking with an open fd into a paused FUSE filesystem.
        kill(
            Pid::from_raw(self.process.as_ref().unwrap().id() as i32),
            signal,
        )
        .unwrap();
    }
    fn disarm_watchdog(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
    }
    pub fn stop(&mut self) {
        // The watchdog must not retain a numeric pid after we reap the child.
        self.disarm_watchdog();
        self.signal(nix::sys::signal::Signal::SIGTERM);
        self.wait_exit();
    }
    pub fn wait_exit(&mut self) {
        self.disarm_watchdog();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() {
                assert!(status.success(), "mount exit {status}");
                break;
            }
            assert!(Instant::now() < deadline, "unmount did not finish");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!mounted(&self.path));
        self.process.take();
    }
}
impl Drop for TempMount {
    fn drop(&mut self) {
        self.disarm_watchdog();
        let unmount = Command::new("fusermount3")
            .arg("-u")
            .arg(&self.path)
            .output();
        if !unmount.is_ok_and(|s| s.status.success()) {
            let _ = Command::new("fusermount3")
                .arg("-uz")
                .arg(&self.path)
                .output();
        }
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(session) = self.session.take() {
            let _ = session.umount_and_join();
        }
        if let (Some(rt), Some(mount)) = (&self.runtime, &self.mount) {
            rt.block_on(mount.shutdown());
        }
        self.runtime.take();
        let _ = std::fs::remove_dir(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("log"));
        if !std::thread::panicking() {
            assert!(
                !self.expired.load(Ordering::SeqCst),
                "mount test watchdog expired"
            );
        }
    }
}
pub fn on_mount<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(f).join().unwrap())
}
