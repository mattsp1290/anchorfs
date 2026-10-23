//! Worker pod lifecycle: template substitution from ConfigMap `anchorfs-worker-template`,
//! readiness and FUSE-entry waits with G4 diagnostics, and the quiesced delete protocol.
use crate::{
    api::SessionRef,
    kube::{Kubectl, LogCapture, MAX_WAIT, PodWatch, log_has_event},
};
use serde_json::Value;
use std::{
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const WORKSPACE_PATH: &str = "/workspace";
/// The template's `terminationGracePeriodSeconds`.
pub const GRACE: Duration = Duration::from_secs(30);
const FUSE_WAIT: Duration = Duration::from_secs(120);
/// How long `delete_pod` waits for its watch and log follower to attach before deleting.
const ATTACH_WAIT: Duration = Duration::from_secs(15);
/// Marker carried in the command line of the worker-side loop the test starts. The kill snippet
/// matches it as `anchorfs-proof-[t]ick`, which does not match the snippet's own command line.
pub const LOOP_MARKER: &str = "anchorfs-proof-tick";

pub struct WorkerSpec<'a> {
    pub name: &'a str,
    pub node: &'a str,
    pub workspace: Uuid,
    pub session: &'a SessionRef,
    pub credential_secret: &'a str,
    /// Workflow Secret name (key `token`) and the lease mutation epoch for a bound successor.
    pub workflow: Option<(&'a str, i64)>,
}

#[derive(Clone, Debug)]
pub struct WorkerPod {
    pub name: String,
    pub node: String,
}

/// One container's spec image reference and the image ID the runtime reports for it.
pub struct ContainerImage {
    pub name: String,
    pub image: String,
    pub image_id: String,
}

pub struct PodFacts {
    pub node_name: String,
    pub uid: String,
    pub image_ids: Vec<String>,
    pub containers: Vec<ContainerImage>,
}

pub struct DeleteReport {
    pub mount_exit_code: i64,
    pub removed_after: Duration,
}

pub struct Pods {
    pub kubectl: Kubectl,
    pub run: String,
    template: String,
}

pub fn run_selector(run: &str) -> String {
    format!("anchorfs.proof/run={run}")
}

/// Fails with the contract text `blocked: node <name>: <stage>`.
pub fn blocked(node: &str, stage: &str) -> ! {
    panic!("blocked: node {node}: {stage}");
}

impl Pods {
    /// Cheap and infallible; the ConfigMap is read by `load_template` inside the proof's
    /// panic guard so a missing template is recorded as a failed phase in the evidence.
    pub fn new(kubectl: &Kubectl, run: &str) -> Self {
        Self {
            kubectl: kubectl.clone(),
            run: run.into(),
            template: String::new(),
        }
    }

    /// Reads `pod.yaml` from ConfigMap `anchorfs-worker-template`.
    pub fn load_template(&mut self) {
        let config = self
            .kubectl
            .get_json(&["configmap", "anchorfs-worker-template"]);
        self.template = config["data"]["pod.yaml"]
            .as_str()
            .expect("ConfigMap anchorfs-worker-template has no pod.yaml")
            .to_owned();
    }

    /// Plain string substitution of every placeholder plus the run label.
    pub fn render(&self, spec: &WorkerSpec<'_>) -> String {
        let (workflow_secret, epoch_args) = match spec.workflow {
            Some((secret, epoch)) => (
                secret.to_owned(),
                format!(
                    "\"--workflow-epoch\", \"{epoch}\", \"--workflow-token-file\", \"/etc/anchorfs/workflow/token\""
                ),
            ),
            None => (spec.credential_secret.to_owned(), String::new()),
        };
        assert!(!self.template.is_empty(), "worker template not loaded");
        let mut text = self.template.clone();
        for (placeholder, value) in [
            ("__NAME__", spec.name.to_owned()),
            ("__NODE__", spec.node.to_owned()),
            ("__WORKSPACE__", spec.workspace.to_string()),
            ("__SESSION_ID__", spec.session.session_id.to_string()),
            ("__SESSION_EPOCH__", spec.session.session_epoch.to_string()),
            ("__CREDENTIAL_SECRET__", spec.credential_secret.to_owned()),
            ("__WORKFLOW_SECRET__", workflow_secret),
            ("__WORKFLOW_EPOCH_ARGS__", epoch_args),
        ] {
            text = text.replace(placeholder, &value);
        }
        assert!(
            !has_placeholder(&text),
            "worker template still contains a placeholder after substitution"
        );
        add_run_label(&text, &self.run)
    }

    /// Applies the pod, waits for readiness of both containers and for the FUSE entry.
    pub fn create(&self, spec: &WorkerSpec<'_>) -> WorkerPod {
        let pod = WorkerPod {
            name: spec.name.into(),
            node: spec.node.into(),
        };
        self.kubectl.apply(&self.render(spec));
        let resource = format!("pod/{}", pod.name);
        if !self.kubectl.wait_condition(&resource, "Ready", MAX_WAIT) {
            self.diagnostics(&pod);
            blocked(&pod.node, "pod not Ready");
        }
        let status = self.kubectl.get_json(&["pod", &pod.name]);
        let ready = status["status"]["containerStatuses"]
            .as_array()
            .is_some_and(|all| all.len() == 2 && all.iter().all(|c| c["ready"] == true));
        if !ready {
            self.diagnostics(&pod);
            blocked(&pod.node, "both containers not ready");
        }
        if !self.wait_fuse(&pod) {
            self.diagnostics(&pod);
            blocked(&pod.node, "no fuse entry at /workspace in worker mountinfo");
        }
        pod
    }

    fn wait_fuse(&self, pod: &WorkerPod) -> bool {
        let deadline = Instant::now() + FUSE_WAIT;
        while Instant::now() < deadline {
            let output = self.kubectl.exec_sh(
                &pod.name,
                "worker",
                "cat /proc/self/mountinfo",
                Duration::from_secs(20),
            );
            if output.success() && has_fuse_entry(&output.stdout, WORKSPACE_PATH) {
                return true;
            }
            thread::sleep(Duration::from_secs(2));
        }
        false
    }

    /// G4 diagnostics, printed rather than returned so a failure carries them in test output.
    pub fn diagnostics(&self, pod: &WorkerPod) {
        eprintln!(
            "--- diagnostics for pod {} on node {} ---",
            pod.name, pod.node
        );
        let steps: [(&str, &str, &str); 4] = [
            (
                "mount",
                "fusermount3 setuid bit",
                "ls -l $(command -v fusermount3)",
            ),
            (
                "mount",
                "root propagation",
                "findmnt -o TARGET,PROPAGATION /",
            ),
            (
                "mount",
                "mount container mountinfo",
                "cat /proc/self/mountinfo",
            ),
            (
                "worker",
                "worker container mountinfo",
                "cat /proc/self/mountinfo",
            ),
        ];
        for (container, title, script) in steps {
            let output =
                self.kubectl
                    .exec_sh(&pod.name, container, script, Duration::from_secs(20));
            eprintln!(
                "[{title}] code={:?}\n{}{}",
                output.code,
                output.stdout.trim_end(),
                output.stderr.trim_end()
            );
        }
        eprintln!("[mount log]\n{}", self.kubectl.logs(&pod.name, "mount"));
        let described = self
            .kubectl
            .run(&["describe", "pod", &pod.name], Duration::from_secs(30));
        eprintln!("[describe]\n{}", described.stdout);
    }

    pub fn facts(&self, pod: &WorkerPod) -> PodFacts {
        let status = self.kubectl.get_json(&["pod", &pod.name]);
        let statuses = status["status"]["containerStatuses"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let containers = status["spec"]["containers"]
            .as_array()
            .map(|all| {
                all.iter()
                    .map(|c| {
                        let name = c["name"].as_str().unwrap_or("").to_owned();
                        let image_id = statuses
                            .iter()
                            .find(|s| s["name"].as_str() == Some(name.as_str()))
                            .and_then(|s| s["imageID"].as_str())
                            .unwrap_or("")
                            .to_owned();
                        ContainerImage {
                            image: c["image"].as_str().unwrap_or("").to_owned(),
                            name,
                            image_id,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        PodFacts {
            containers,
            node_name: status["spec"]["nodeName"].as_str().unwrap_or("").to_owned(),
            uid: status["metadata"]["uid"].as_str().unwrap_or("").to_owned(),
            image_ids: status["status"]["containerStatuses"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(|c| c["imageID"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn exec(&self, pod: &WorkerPod, script: &str) -> crate::kube::Output {
        self.kubectl
            .exec_sh(&pod.name, "worker", script, Duration::from_secs(120))
    }

    /// Starts the tick loop (appends to `file` every 0.5 s) as a detached worker-side process.
    pub fn start_tick_loop(&self, pod: &WorkerPod, file: &str) {
        let script = format!(
            "nohup sh -c 'while :; do echo tick >> {file}; sleep 0.5; done # {LOOP_MARKER}' >/dev/null 2>&1 &"
        );
        let output = self.exec(pod, &script);
        assert!(
            output.success(),
            "starting tick loop failed: {}",
            output.stderr
        );
    }

    /// Kills the known worker-side loop. The pattern `[t]ick` keeps the snippet from matching
    /// its own command line.
    pub fn kill_loops(&self, pod: &WorkerPod) {
        self.kill_loops_in(&pod.name, Duration::from_secs(120));
    }

    fn kill_loops_in(&self, pod_name: &str, timeout: Duration) {
        let marker = LOOP_MARKER.replace("-tick", "-[t]ick");
        let script = format!(
            "for p in /proc/[0-9]*; do pid=${{p#/proc/}}; [ \"$pid\" = \"$$\" ] && continue; \
             if tr '\\0' ' ' < $p/cmdline 2>/dev/null | grep -q '{marker}'; then kill $pid; fi; done; true"
        );
        let _ = self.kubectl.exec_sh(pod_name, "worker", &script, timeout);
    }

    /// Step 1 of `delete_pod`: no worker-side process may hold a cwd or fd under /workspace,
    /// because the propagated mount copy would make the sidecar's non-lazy unmount fail.
    pub fn quiesce(&self, pod: &WorkerPod) {
        self.kill_loops(pod);
        let script = format!(
            "ls -l /proc/[0-9]*/cwd /proc/[0-9]*/fd 2>/dev/null | grep {WORKSPACE_PATH}; true"
        );
        for _ in 0..5 {
            thread::sleep(Duration::from_secs(1));
            let output = self.exec(pod, &script);
            if output.success() && output.stdout.trim().is_empty() {
                return;
            }
        }
        let held = self.exec(pod, &script).stdout;
        panic!(
            "quiesce failed on node {}: worker processes still reference {WORKSPACE_PATH}:\n{held}",
            pod.node
        );
    }

    /// Quiesce, capture the sidecar log, delete without waiting, require a clean sidecar exit
    /// and the `mount shut down` event, then wait for the pod object to disappear.
    ///
    /// The worker traps SIGTERM and exits within about a second, so the window in which
    /// `state.terminated` is visible to a poller can be shorter than one poll interval, and a
    /// `kubectl logs -f` that connects after the delete can miss the pod entirely. Both
    /// observers are therefore attached BEFORE the delete: a `kubectl get pod --watch
    /// --output-watch-events` stream (every status update, including the one carrying the
    /// sidecar's `terminated` state, arrives before the final `DELETED` event) and the log
    /// follower, which is confirmed to have received bytes first. "blocked" keeps its meaning:
    /// no terminated state for the sidecar before the pod is gone or the bound elapses, or the
    /// object still present after the bound.
    pub fn delete_pod(&self, pod: &WorkerPod) -> DeleteReport {
        self.quiesce(pod);
        let capture = LogCapture::start(&self.kubectl, &pod.name, "mount");
        let watch = PodWatch::start(&self.kubectl, &pod.name);
        let attach_deadline = Instant::now() + ATTACH_WAIT;
        while Instant::now() < attach_deadline
            && (watch.events().is_empty() || capture.text().is_empty())
        {
            thread::sleep(Duration::from_millis(200));
        }
        if watch.events().is_empty() {
            blocked(&pod.node, "pod watch did not attach before the delete");
        }
        if capture.text().is_empty() {
            eprintln!(
                "node {}: sidecar log capture received no bytes before the delete; the shutdown event check below decides",
                pod.node
            );
        }
        let started = Instant::now();
        let deleted = self.kubectl.run(
            &["delete", "pod", &pod.name, "--wait=false"],
            Duration::from_secs(30),
        );
        assert!(
            deleted.success(),
            "pod delete failed: {}",
            deleted.stderr.trim()
        );
        let deadline = started + GRACE + Duration::from_secs(10);
        let mut seen = MountEvents::default();
        let mut removed_after = None;
        while Instant::now() < deadline {
            seen = scan_mount_events(&watch.events());
            if seen.deleted {
                removed_after = Some(started.elapsed());
            }
            if seen.exit_code.is_some() || seen.deleted {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        let Some(exit_code) = seen.exit_code else {
            blocked(
                &pod.node,
                "mount container never reported terminated state (see docs/ops/cluster-proof.md recovery)",
            );
        };
        assert_eq!(
            exit_code, 0,
            "node {}: mount sidecar exit code {exit_code}, expected 0",
            pod.node
        );
        let log = capture.finish(Duration::from_secs(10));
        assert!(
            log_has_event(&log, "mount shut down", None),
            "node {}: sidecar log lacks the \"mount shut down\" event:\n{log}",
            pod.node
        );
        while removed_after.is_none() && Instant::now() < deadline {
            if scan_mount_events(&watch.events()).deleted {
                removed_after = Some(started.elapsed());
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        match removed_after {
            Some(removed_after) => DeleteReport {
                mount_exit_code: exit_code,
                removed_after,
            },
            None => blocked(
                &pod.node,
                "pod stuck Terminating (see docs/ops/cluster-proof.md recovery)",
            ),
        }
    }

    /// Best-effort teardown of every object carrying this run's label.
    pub fn cleanup(&self) {
        let selector = run_selector(&self.run);
        // Quiesce first: a tick loop still appending under /workspace would make the sidecar's
        // non-lazy unmount fail and leave a dead FUSE mount on the node after SIGKILL.
        if let Some(list) = self.kubectl.try_get_json(&["pods", "-l", &selector]) {
            for item in list["items"].as_array().into_iter().flatten() {
                if let Some(name) = item["metadata"]["name"].as_str() {
                    self.kill_loops_in(name, Duration::from_secs(20));
                }
            }
        }
        self.kubectl.delete_by_label("pod,secret", &selector);
    }
}

/// What a pod watch stream has shown about the `mount` container.
#[derive(Default)]
struct MountEvents {
    /// Exit code of the sidecar's `state.terminated`, from the first event that carried one.
    exit_code: Option<i64>,
    /// A `DELETED` event was received.
    deleted: bool,
}

fn scan_mount_events(events: &[Value]) -> MountEvents {
    let mut seen = MountEvents::default();
    for event in events {
        if event["type"] == "DELETED" {
            seen.deleted = true;
        }
        if seen.exit_code.is_some() {
            continue;
        }
        let terminated = event["object"]["status"]["containerStatuses"]
            .as_array()
            .and_then(|all| all.iter().find(|c| c["name"] == "mount"))
            .map(|c| &c["state"]["terminated"]);
        if let Some(state) = terminated.filter(|state| state.is_object()) {
            seen.exit_code = state["exitCode"].as_i64();
        }
    }
    seen
}

fn has_placeholder(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(offset) = text[start..].find("__") {
        let at = start + offset + 2;
        let word: usize = bytes[at.min(bytes.len())..]
            .iter()
            .take_while(|b| b.is_ascii_uppercase() || **b == b'_')
            .count();
        if word >= 2 && text[at..at + word].contains(|c: char| c.is_ascii_uppercase()) {
            // `__NAME__` style: uppercase letters then a closing `__`.
            if text[at..at + word].ends_with("__") {
                return true;
            }
        }
        start = at;
    }
    false
}

fn add_run_label(manifest: &str, run: &str) -> String {
    let marker = "anchorfs.proof/role: worker";
    let mut out = String::with_capacity(manifest.len() + 64);
    let mut added = false;
    for line in manifest.lines() {
        out.push_str(line);
        out.push('\n');
        if !added && line.trim_start().starts_with(marker) {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}anchorfs.proof/run: \"{run}\"\n"));
            added = true;
        }
    }
    assert!(added, "worker template lacks the anchorfs.proof/role label");
    out
}

/// True when mountinfo has a `fuse`/`fuse.*` filesystem mounted at `path`.
pub fn has_fuse_entry(mountinfo: &str, path: &str) -> bool {
    mountinfo.lines().any(|line| {
        let Some((head, tail)) = line.split_once(" - ") else {
            return false;
        };
        let mount_point = head.split(' ').nth(4);
        let fs_type = tail.split(' ').next().unwrap_or("");
        mount_point == Some(path) && (fs_type == "fuse" || fs_type.starts_with("fuse."))
    })
}

#[test]
fn fuse_entry_parser_requires_fuse_at_the_workspace() {
    let line = "812 790 0:99 / /workspace rw,nosuid - fuse anchorfs rw,user_id=10001";
    assert!(has_fuse_entry(line, "/workspace"));
    assert!(!has_fuse_entry(line, "/tmp"));
    let tmpfs = "812 790 0:99 / /workspace rw - tmpfs tmpfs rw";
    assert!(!has_fuse_entry(tmpfs, "/workspace"));
}

#[test]
fn placeholder_scan_finds_only_template_tokens() {
    assert!(has_placeholder("name: __NAME__"));
    assert!(has_placeholder("x __WORKFLOW_EPOCH_ARGS__]"));
    assert!(!has_placeholder("name: anchorfs-proof my_var __init"));
}

#[test]
fn mount_events_report_terminated_state_and_deletion_in_order() {
    let running = serde_json::json!({"type": "ADDED", "object": {"status": {"containerStatuses": [
        {"name": "mount", "state": {"running": {}}}]}}});
    let terminated = serde_json::json!({"type": "MODIFIED", "object": {"status": {"containerStatuses": [
        {"name": "worker", "state": {"terminated": {"exitCode": 9}}},
        {"name": "mount", "state": {"terminated": {"exitCode": 0}}}]}}});
    let gone = serde_json::json!({"type": "DELETED", "object": {}});
    let early = scan_mount_events(std::slice::from_ref(&running));
    assert!(early.exit_code.is_none() && !early.deleted);
    let seen = scan_mount_events(&[running, terminated, gone.clone()]);
    assert_eq!(
        seen.exit_code,
        Some(0),
        "the worker's exit code must not count"
    );
    assert!(seen.deleted);
    let vanished = scan_mount_events(&[gone]);
    assert!(vanished.exit_code.is_none() && vanished.deleted);
}

/// Fixture with the two template lines whose substitution the cluster run depends on: the
/// mount `args` flow sequence ending in `__WORKFLOW_EPOCH_ARGS__` (a trailing comma in the
/// ordinary variant, which libyaml-derived parsers such as kubectl's accept) and the label
/// the run label is attached to.
#[cfg(test)]
const FIXTURE_TEMPLATE: &str = "apiVersion: v1
kind: Pod
metadata:
  name: __NAME__
  labels:
    anchorfs.proof/role: worker
spec:
  nodeName: __NODE__
  volumes:
    - name: credential
      secret:
        secretName: __CREDENTIAL_SECRET__
    - name: workflow
      secret:
        secretName: __WORKFLOW_SECRET__
        optional: true
  containers:
    - name: mount
      args: [\"mount\", \"/workspace\", \"--workspace\", \"__WORKSPACE__\", \"--session-id\", \"__SESSION_ID__\", \"--session-epoch\", \"__SESSION_EPOCH__\", \"--log-format\", \"json\", __WORKFLOW_EPOCH_ARGS__]
";

/// The `args: [...]` flow sequence of a rendered manifest as JSON, tolerating the single
/// trailing comma YAML flow sequences allow.
#[cfg(test)]
fn rendered_args(manifest: &str) -> Vec<String> {
    let line = manifest
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("args: "))
        .expect("args line");
    let inner = line
        .trim()
        .strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .expect("flow sequence brackets");
    let inner = inner.trim_end();
    let inner = inner.strip_suffix(',').unwrap_or(inner);
    let parsed: Vec<String> =
        serde_json::from_str(&format!("[{inner}]")).expect("args parse as a flow sequence");
    parsed
}

#[cfg(test)]
fn fixture_pods() -> Pods {
    let mut pods = Pods::new(&Kubectl::new("kubectl", "/nonexistent", "ns"), "run-1");
    pods.template = FIXTURE_TEMPLATE.to_owned();
    pods
}

#[test]
fn substituted_worker_template_parses_for_ordinary_and_bound_pods() {
    let pods = fixture_pods();
    let session = SessionRef {
        session_id: Uuid::nil(),
        session_epoch: 3,
    };
    let ordinary = pods.render(&WorkerSpec {
        name: "pod-a",
        node: "node-1",
        workspace: Uuid::nil(),
        session: &session,
        credential_secret: "cred-1",
        workflow: None,
    });
    let args = rendered_args(&ordinary);
    assert_eq!(args.first().map(String::as_str), Some("mount"));
    assert_eq!(args.last().map(String::as_str), Some("json"));
    assert!(!args.iter().any(|a| a.starts_with("--workflow")));
    assert!(ordinary.contains("secretName: cred-1"));
    assert!(ordinary.contains("anchorfs.proof/run: \"run-1\""));
    assert!(!has_placeholder(&ordinary));

    let bound = pods.render(&WorkerSpec {
        name: "pod-d",
        node: "node-2",
        workspace: Uuid::nil(),
        session: &session,
        credential_secret: "cred-2",
        workflow: Some(("wf-secret", 7)),
    });
    let args = rendered_args(&bound);
    let tail: Vec<&str> = args
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        [
            "--workflow-epoch",
            "7",
            "--workflow-token-file",
            "/etc/anchorfs/workflow/token"
        ]
    );
    assert!(bound.contains("secretName: wf-secret"));
    assert!(bound.contains("secretName: cred-2"));
}
