//! Worker pod lifecycle: template substitution from ConfigMap `anchorfs-worker-template`,
//! readiness and FUSE-entry waits with G4 diagnostics, and the quiesced delete protocol.
use crate::kube::{Kubectl, LogCapture, MAX_WAIT, PodWatch, log_has_event};
use anchorfs_client::auth::SessionAuthority;
use serde_json::{Value, json};
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
    pub session: &'a SessionAuthority,
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
    pub containers: Vec<ContainerImage>,
}

pub struct DeleteReport {
    pub mount_exit_code: i64,
    pub removed_after: Duration,
}

pub struct Pods {
    pub kubectl: Kubectl,
    pub run: String,
    /// `pod.yaml` of ConfigMap `anchorfs-worker-template`.
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
    pub fn new(kubectl: &Kubectl, run: &str, template: String) -> Self {
        Self {
            kubectl: kubectl.clone(),
            run: run.into(),
            template,
        }
    }

    /// Substitutes the scalar placeholders, converts the manifest once to JSON with kubectl
    /// and edits the parsed object (run label, optional workflow arguments and Secret volume).
    pub fn render(&self, spec: &WorkerSpec<'_>) -> Value {
        let mut text = self.template.clone();
        for (placeholder, value) in [
            ("__NAME__", spec.name.to_owned()),
            ("__NODE__", spec.node.to_owned()),
            ("__WORKSPACE__", spec.workspace.to_string()),
            ("__SESSION_ID__", spec.session.session_id.to_string()),
            ("__SESSION_EPOCH__", spec.session.session_epoch.to_string()),
            ("__CREDENTIAL_SECRET__", spec.credential_secret.to_owned()),
        ] {
            text = text.replace(placeholder, &value);
        }
        let mut pod = self.kubectl.to_json(&text);
        if let Some(left) = leftover_placeholder(&pod) {
            panic!("worker template still contains a placeholder after substitution: {left}");
        }
        edit_pod(&mut pod, &self.run, spec.workflow);
        pod
    }

    /// Applies the pod, waits for readiness of both containers and for the FUSE entry.
    pub fn create(&self, spec: &WorkerSpec<'_>) -> WorkerPod {
        let pod = WorkerPod {
            name: spec.name.into(),
            node: spec.node.into(),
        };
        self.kubectl.apply(&self.render(spec).to_string());
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

    /// Step 1 of `delete_pod`: no worker-side process may hold a cwd or fd under /workspace,
    /// because the propagated mount copy would make the sidecar's non-lazy unmount fail.
    pub fn quiesce(&self, pod: &WorkerPod) {
        kill_loops(&self.kubectl, &pod.name, Duration::from_secs(120));
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
        while Instant::now() < deadline && (seen.exit_code.is_none() || removed_after.is_none()) {
            seen = scan_mount_events(&watch.events());
            if seen.deleted && removed_after.is_none() {
                removed_after = Some(started.elapsed());
            }
            // A vanished pod will report nothing further.
            if seen.deleted && seen.exit_code.is_none() {
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
}

/// Kills the known worker-side loop. The pattern `[t]ick` keeps the snippet from matching
/// its own command line.
pub fn kill_loops(kubectl: &Kubectl, pod_name: &str, timeout: Duration) {
    let marker = LOOP_MARKER.replace("-tick", "-[t]ick");
    let script = format!(
        "for p in /proc/[0-9]*; do pid=${{p#/proc/}}; [ \"$pid\" = \"$$\" ] && continue; \
         if tr '\\0' ' ' < $p/cmdline 2>/dev/null | grep -q '{marker}'; then kill $pid; fi; done; true"
    );
    let _ = kubectl.exec_sh(pod_name, "worker", &script, timeout);
}

/// Best-effort teardown of every object carrying this run's label.
pub fn cleanup_run(kubectl: &Kubectl, run: &str) {
    let selector = run_selector(run);
    // Quiesce first: a tick loop still appending under /workspace would make the sidecar's
    // non-lazy unmount fail and leave a dead FUSE mount on the node after SIGKILL.
    if let Some(list) = kubectl.try_get_json(&["pods", "-l", &selector]) {
        for item in list["items"].as_array().into_iter().flatten() {
            if let Some(name) = item["metadata"]["name"].as_str() {
                kill_loops(kubectl, name, Duration::from_secs(20));
            }
        }
    }
    kubectl.delete_by_label("pod,secret", &selector);
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

/// First string in the manifest that still holds an unsubstituted `__PLACEHOLDER__` marker.
fn leftover_placeholder(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if text.contains("__") => Some(text.clone()),
        Value::Array(all) => all.iter().find_map(leftover_placeholder),
        Value::Object(map) => map.values().find_map(leftover_placeholder),
        _ => None,
    }
}

/// Adds the run label and, for a pod bound to a workflow lease, the workflow arguments and the
/// Secret volume (key `token`) with its read-only mount on the `mount` container.
fn edit_pod(pod: &mut Value, run: &str, workflow: Option<(&str, i64)>) {
    pod["metadata"]["labels"]["anchorfs.proof/run"] = json!(run);
    let Some((secret, epoch)) = workflow else {
        return;
    };
    let mount = pod["spec"]["containers"]
        .as_array_mut()
        .and_then(|all| all.iter_mut().find(|c| c["name"] == "mount"))
        .expect("worker template has a mount container");
    mount["args"]
        .as_array_mut()
        .expect("mount container args")
        .extend(
            [
                "--workflow-epoch",
                &epoch.to_string(),
                "--workflow-token-file",
                "/etc/anchorfs/workflow/token",
            ]
            .map(Value::from),
        );
    mount["volumeMounts"]
        .as_array_mut()
        .expect("mount container volumeMounts")
        .push(json!({"name": "workflow", "mountPath": "/etc/anchorfs/workflow", "readOnly": true}));
    pod["spec"]["volumes"]
        .as_array_mut()
        .expect("pod volumes")
        .push(json!({"name": "workflow", "secret": {"secretName": secret, "defaultMode": 288}}));
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

#[test]
fn pod_edits_label_ordinary_pods_and_extend_bound_ones() {
    let pod = json!({
        "metadata": {"labels": {"anchorfs.proof/role": "worker"}},
        "spec": {
            "volumes": [{"name": "credential", "secret": {"secretName": "cred-1"}}],
            "containers": [
                {"name": "mount", "args": ["mount", "--log-format", "json"], "volumeMounts": []},
                {"name": "worker"}
            ]
        }
    });
    let mut ordinary = pod.clone();
    edit_pod(&mut ordinary, "run-1", None);
    assert_eq!(
        ordinary["metadata"]["labels"]["anchorfs.proof/run"],
        "run-1"
    );
    assert_eq!(
        ordinary["metadata"]["labels"]["anchorfs.proof/role"],
        "worker"
    );
    assert_eq!(
        ordinary["spec"]["containers"][0]["args"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(
        ordinary["spec"]["volumes"].as_array().map(Vec::len),
        Some(1)
    );

    let mut bound = pod;
    edit_pod(&mut bound, "run-1", Some(("wf-secret", 7)));
    assert_eq!(
        bound["spec"]["containers"][0]["args"],
        json!([
            "mount",
            "--log-format",
            "json",
            "--workflow-epoch",
            "7",
            "--workflow-token-file",
            "/etc/anchorfs/workflow/token"
        ])
    );
    assert_eq!(
        bound["spec"]["volumes"][1]["secret"]["secretName"],
        "wf-secret"
    );
    assert_eq!(
        bound["spec"]["volumes"][0]["secret"]["secretName"],
        "cred-1"
    );
    assert_eq!(
        bound["spec"]["containers"][0]["volumeMounts"][0]["name"],
        "workflow"
    );
    assert_eq!(leftover_placeholder(&bound), None);
    assert!(leftover_placeholder(&json!({"a": ["x __NAME__"]})).is_some());
}
