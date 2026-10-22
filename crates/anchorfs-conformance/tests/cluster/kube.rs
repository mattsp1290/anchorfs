//! Thin `kubectl` wrappers over `std::process::Command`: bounded runtime, captured output,
//! port-forward child killed on drop, and Secret creation from an owner-only temp file.
//! Bearer and lease values never appear in argv, error text or logs.
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::OpenOptionsExt,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// Longest single wait the plan allows.
pub const MAX_WAIT: Duration = Duration::from_secs(180);

pub struct Output {
    /// `None` when the child was killed for exceeding its timeout or was signalled.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}
impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
}

/// Runs a prepared command with captured output and a hard timeout.
pub fn run_command(mut command: Command, stdin: Option<&[u8]>, timeout: Duration) -> Output {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Output {
                code: None,
                stdout: String::new(),
                stderr: format!("spawn failed: {error}"),
                timed_out: false,
            };
        }
    };
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let bytes = bytes.to_vec();
        thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(_) => break None,
        }
    };
    Output {
        code: status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
        timed_out,
    }
}
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buffer);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

#[derive(Clone)]
pub struct Kubectl {
    program: Vec<String>,
    kubeconfig: String,
    pub namespace: String,
}
impl Kubectl {
    /// `command` is split on whitespace, so `k3s kubectl` works.
    pub fn new(command: &str, kubeconfig: &str, namespace: &str) -> Self {
        let program: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
        assert!(!program.is_empty(), "ANCHORFS_CLUSTER_KUBECTL is empty");
        Self {
            program,
            kubeconfig: kubeconfig.into(),
            namespace: namespace.into(),
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program[0]);
        command
            .args(&self.program[1..])
            .arg("-n")
            .arg(&self.namespace)
            .args(args)
            .env("KUBECONFIG", &self.kubeconfig);
        command
    }
    pub fn run(&self, args: &[&str], timeout: Duration) -> Output {
        run_command(self.command(args), None, timeout)
    }
    pub fn run_stdin(&self, args: &[&str], stdin: &[u8], timeout: Duration) -> Output {
        run_command(self.command(args), Some(stdin), timeout)
    }
    /// stdout of a successful invocation; panics with kubectl's stderr otherwise.
    pub fn ok(&self, args: &[&str], timeout: Duration) -> String {
        let output = self.run(args, timeout);
        assert!(
            output.success(),
            "kubectl {} failed (code {:?}, timed_out {}): {}",
            args.first().copied().unwrap_or(""),
            output.code,
            output.timed_out,
            output.stderr.trim()
        );
        output.stdout
    }
    pub fn get_json(&self, args: &[&str]) -> Value {
        let mut full = vec!["get"];
        full.extend_from_slice(args);
        full.extend_from_slice(&["-o", "json"]);
        let text = self.ok(&full, Duration::from_secs(30));
        serde_json::from_str(&text).expect("kubectl produced JSON")
    }
    pub fn try_get_json(&self, args: &[&str]) -> Option<Value> {
        let mut full = vec!["get"];
        full.extend_from_slice(args);
        full.extend_from_slice(&["-o", "json", "--ignore-not-found"]);
        let output = self.run(&full, Duration::from_secs(30));
        if !output.success() || output.stdout.trim().is_empty() {
            return None;
        }
        serde_json::from_str(&output.stdout).ok()
    }
    pub fn apply(&self, manifest: &str) {
        let output = self.run_stdin(
            &["apply", "-f", "-"],
            manifest.as_bytes(),
            Duration::from_secs(60),
        );
        assert!(
            output.success(),
            "kubectl apply failed: {}",
            output.stderr.trim()
        );
    }
    pub fn wait_condition(&self, resource: &str, condition: &str, timeout: Duration) -> bool {
        let timeout = timeout.min(MAX_WAIT);
        let flag = format!("--for=condition={condition}");
        let limit = format!("--timeout={}s", timeout.as_secs());
        self.run(
            &["wait", &flag, &limit, resource],
            timeout + Duration::from_secs(15),
        )
        .success()
    }
    /// `kubectl exec -c <container> -- sh -c <script>`; the script must not embed secrets.
    pub fn exec_sh(&self, pod: &str, container: &str, script: &str, timeout: Duration) -> Output {
        self.run(
            &["exec", pod, "-c", container, "--", "sh", "-c", script],
            timeout,
        )
    }
    pub fn logs(&self, pod: &str, container: &str) -> String {
        let output = self.run(&["logs", pod, "-c", container], Duration::from_secs(30));
        format!("{}{}", output.stdout, output.stderr)
    }
    pub fn delete_by_label(&self, kinds: &str, selector: &str) {
        let _ = self.run(
            &[
                "delete",
                kinds,
                "-l",
                selector,
                "--wait=false",
                "--ignore-not-found",
            ],
            Duration::from_secs(60),
        );
    }
    /// Creates (or replaces) a generic Secret with one key. The value travels through a 0600
    /// temp file that is removed afterwards, never through argv.
    pub fn create_secret(&self, name: &str, key: &str, value: &str, labels: &[(&str, &str)]) {
        let file = SecretFile::create(value);
        let from_file = format!("--from-file={key}={}", file.path());
        let _ = self.run(
            &["delete", "secret", name, "--ignore-not-found"],
            Duration::from_secs(30),
        );
        let output = self.run(
            &["create", "secret", "generic", name, &from_file],
            Duration::from_secs(30),
        );
        assert!(
            output.success(),
            "creating secret {name} failed: {}",
            output.stderr.trim()
        );
        for (label_key, label_value) in labels {
            let label = format!("{label_key}={label_value}");
            let _ = self.run(
                &["label", "secret", name, &label, "--overwrite"],
                Duration::from_secs(30),
            );
        }
    }
    /// Reads one key of a Secret as bytes. The result is returned to the caller only.
    pub fn secret_value(&self, name: &str, key: &str) -> Vec<u8> {
        let secret = self.get_json(&["secret", name]);
        let encoded = secret["data"][key]
            .as_str()
            .unwrap_or_else(|| panic!("secret {name} has no key {key}"));
        base64_decode(encoded)
    }
    pub fn secret_keys(&self, name: &str) -> Vec<String> {
        let secret = self.get_json(&["secret", name]);
        secret["data"]
            .as_object()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }
    pub fn version_line(&self) -> String {
        let output = self.run(&["version", "--client"], Duration::from_secs(20));
        output.stdout.lines().next().unwrap_or("").trim().to_owned()
    }
    pub fn port_forward(&self, target: &str, remote_port: u16) -> PortForward {
        PortForward::start(self.clone(), target, remote_port)
    }
}

/// Owner-only temp file removed on drop.
struct SecretFile(std::path::PathBuf);
impl SecretFile {
    fn create(value: &str) -> Self {
        let path = std::env::temp_dir().join(format!("anchorfs-cluster-{}", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .expect("secret temp file");
        file.write_all(value.as_bytes()).expect("secret temp write");
        Self(path)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut accumulator, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    out
}

/// `kubectl port-forward` child bound to a free loopback port; killed on drop.
pub struct PortForward {
    kubectl: Kubectl,
    target: String,
    remote_port: u16,
    pub local_port: u16,
    child: Option<Child>,
    stderr: Arc<Mutex<String>>,
}
impl PortForward {
    fn start(kubectl: Kubectl, target: &str, remote_port: u16) -> Self {
        let local_port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("free local port");
            listener.local_addr().expect("local addr").port()
        };
        let mut forward = Self {
            kubectl,
            target: target.into(),
            remote_port,
            local_port,
            child: None,
            stderr: Arc::new(Mutex::new(String::new())),
        };
        forward.spawn();
        forward
    }
    fn spawn(&mut self) {
        self.kill();
        let mapping = format!("{}:{}", self.local_port, self.remote_port);
        let mut command = self.kubectl.command(&[
            "port-forward",
            "--address",
            "127.0.0.1",
            &self.target,
            &mapping,
        ]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn kubectl port-forward");
        if let Some(mut pipe) = child.stderr.take() {
            let sink = Arc::clone(&self.stderr);
            thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                if let Ok(mut guard) = sink.lock() {
                    guard.push_str(&text);
                }
            });
        }
        self.child = Some(child);
    }
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    /// Waits until the local port accepts connections (bounded) and returns whether it does.
    pub fn wait_listening(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout.min(MAX_WAIT);
        let address = SocketAddr::from(([127, 0, 0, 1], self.local_port));
        while Instant::now() < deadline {
            if TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok() {
                return true;
            }
            if self
                .child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
            {
                return false;
            }
            thread::sleep(Duration::from_millis(200));
        }
        false
    }
    /// Restarts the forward on the same local port if the child exited (connection loss).
    pub fn ensure_alive(&mut self) {
        let exited = self
            .child
            .as_mut()
            .is_none_or(|child| matches!(child.try_wait(), Ok(Some(_)) | Err(_)));
        if exited {
            self.spawn();
            self.wait_listening(Duration::from_secs(30));
        }
    }
    pub fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }
}
impl Drop for PortForward {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Streams `kubectl logs -f` into a buffer on background threads; killed on drop.
pub struct LogCapture {
    child: Option<Child>,
    buffer: Arc<Mutex<String>>,
    reader: Option<thread::JoinHandle<()>>,
}
impl LogCapture {
    pub fn start(kubectl: &Kubectl, pod: &str, container: &str) -> Self {
        let mut command = kubectl.command(&["logs", "-f", pod, "-c", container]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("spawn kubectl logs");
        let buffer = Arc::new(Mutex::new(String::new()));
        let reader = child.stdout.take().map(|mut pipe| {
            let sink = Arc::clone(&buffer);
            thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(read) = pipe.read(&mut chunk) {
                    if read == 0 {
                        break;
                    }
                    if let Ok(mut guard) = sink.lock() {
                        guard.push_str(&String::from_utf8_lossy(&chunk[..read]));
                    }
                }
            })
        });
        Self {
            child: Some(child),
            buffer,
            reader,
        }
    }
    pub fn text(&self) -> String {
        self.buffer
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }
    /// Lets the stream drain (the container has terminated) and returns the full capture.
    pub fn finish(mut self, drain: Duration) -> String {
        let deadline = Instant::now() + drain;
        while Instant::now() < deadline
            && self
                .child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
        {
            thread::sleep(Duration::from_millis(100));
        }
        self.stop();
        self.text()
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Drop for LogCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// True when a JSON log line (tracing's json layout) carries the given message, and, when
/// given, the given `reason` field.
pub fn log_has_event(log: &str, message: &str, reason: Option<&str>) -> bool {
    log.lines().any(|line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        let field = |name: &str| {
            value["fields"][name]
                .as_str()
                .or_else(|| value[name].as_str())
                .map(str::to_owned)
        };
        field("message").as_deref() == Some(message)
            && reason.is_none_or(|expected| field("reason").as_deref() == Some(expected))
    })
}
