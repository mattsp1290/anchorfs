//! Phases 0-7 of the two-node cluster proof (plan 03 section 2). Each phase records evidence;
//! any failure records the failing phase, deletes the run's pods and Secrets best-effort and
//! re-raises the original panic.
use crate::{
    api::{Api, Bearer, Lease, SessionRef},
    evidence::Evidence,
    kube::{Kubectl, PortForward, log_has_event},
    pods::{DeleteReport, Pods, WORKSPACE_PATH, WorkerPod, WorkerSpec},
};
use anchorfs_client::{
    Client,
    auth::{Authority, Bearer as ClientBearer, SessionAuthority},
    endpoint::Endpoint,
    readiness::{Readiness, readiness},
    retry::IdempotencyKey,
    wire::OpenHandle,
};
use anchorfs_core::SessionId;
use bytes::Bytes;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const TOTAL_BUDGET: Duration = Duration::from_secs(25 * 60);
const WAIT: Duration = Duration::from_secs(180);
const CHUNK: usize = 1 << 20;
const WORKER_UID: u32 = 10001;

pub struct Config {
    pub kubeconfig: String,
    pub kubectl: String,
    pub namespace: String,
    pub worker_nodes: Vec<String>,
    pub service_node: String,
    pub evidence: String,
}
impl Config {
    pub fn from_env(kubeconfig: String) -> Self {
        let var = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        let worker_nodes: Vec<String> = var(
            "ANCHORFS_CLUSTER_WORKER_NODES",
            "k3s-worker-raspberrypi,k3s-worker-raspberrypi-2",
        )
        .split(',')
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .collect();
        assert!(
            worker_nodes.len() >= 2 && worker_nodes[0] != worker_nodes[1],
            "ANCHORFS_CLUSTER_WORKER_NODES needs two distinct nodes"
        );
        Self {
            kubeconfig,
            kubectl: var("ANCHORFS_CLUSTER_KUBECTL", "k3s kubectl"),
            namespace: var("ANCHORFS_CLUSTER_NAMESPACE", "anchorfs"),
            worker_nodes,
            service_node: var("ANCHORFS_CLUSTER_SERVICE_NODE", "infra-control-master"),
            evidence: var("ANCHORFS_CLUSTER_EVIDENCE", "target/cluster-evidence.json"),
        }
    }
}

struct Proof {
    cfg: Config,
    kubectl: Kubectl,
    runtime: Arc<tokio::runtime::Runtime>,
    run: String,
    started: Instant,
    phase: &'static str,
    evidence: Evidence,
    pods: Pods,
    forward: Option<PortForward>,
    ca: Vec<u8>,
    api: Option<Api>,
    node_arch: Vec<(String, String)>,
    service_restarts: i64,
}

/// Entry point of `two_node_cluster_proof`.
pub fn run(cfg: Config) {
    let kubectl = Kubectl::new(&cfg.kubectl, &cfg.kubeconfig, &cfg.namespace);
    let run = Uuid::new_v4().to_string();
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime"),
    );
    let evidence = Evidence::new(&cfg.evidence, &run, &kubectl.version_line());
    let pods = Pods::new(&kubectl, &run);
    let mut proof = Proof {
        cfg,
        kubectl,
        runtime,
        run,
        started: Instant::now(),
        phase: "start",
        evidence,
        pods,
        forward: None,
        ca: Vec::new(),
        api: None,
        node_arch: Vec::new(),
        service_restarts: 0,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| proof.execute()));
    match outcome {
        Ok(()) => proof.evidence.finish("passed", None),
        Err(panic) => {
            let phase = proof.phase;
            proof.evidence.finish("failed", Some(phase));
            eprintln!("cluster proof failed in phase: {phase}");
            proof.pods.cleanup();
            resume_unwind(panic);
        }
    }
}

struct Authority3 {
    admin: Bearer,
    host: Bearer,
    host_session: SessionRef,
    workspace: Uuid,
    workers: Vec<Bearer>,
    sessions: Vec<SessionRef>,
}

impl Proof {
    fn enter(&mut self, phase: &'static str) {
        self.phase = phase;
        assert!(
            self.started.elapsed() <= TOTAL_BUDGET,
            "blocked: cluster proof exceeded its 25 minute bound before phase {phase}"
        );
        eprintln!(
            "== phase {phase} ({}s elapsed)",
            self.started.elapsed().as_secs()
        );
    }

    fn execute(&mut self) {
        self.preflight();
        let auth = self.authority_setup();
        let client = self.host_client(&auth);
        let (pod_a, digests) = self.case1_harness_to_worker_a(&auth, &client);
        self.case2_replacement_on_other_node(&auth, pod_a, &digests);
        let pod_c = self.case3_idle_then_attach(&auth, &digests);
        let (pod_d, lease) = self.case5_6_fencing_then_successor(&auth, &client, pod_c);
        self.case7_ephemeral_root(&auth, &client, pod_d, lease);
        self.teardown();
    }

    // ---- shared helpers -------------------------------------------------------------

    fn node_of(&self, index: usize) -> String {
        self.cfg.worker_nodes[index].clone()
    }
    fn pod_name(&self, tag: &str) -> String {
        format!("anchorfs-proof-{tag}-{}", &self.run[..8])
    }
    fn secret_name(&self, index: usize) -> String {
        format!("anchorfs-worker-{}-credential", index + 1)
    }
    fn arch_of(&self, node: &str) -> String {
        self.node_arch
            .iter()
            .find(|(name, _)| name == node)
            .map(|(_, arch)| arch.clone())
            .unwrap_or_default()
    }
    fn endpoint(&self) -> Endpoint {
        let port = self.forward.as_ref().expect("port-forward").local_port;
        Endpoint::parse(&format!("https://localhost:{port}"), false).expect("endpoint")
    }
    fn client_for(&self, auth: &Authority3, bearer: &Bearer, session: &SessionRef) -> Client {
        let authority = Authority {
            bearer: ClientBearer::parse(bearer.expose()).expect("bearer"),
            session: Some(SessionAuthority {
                session_id: SessionId(session.session_id),
                session_epoch: session.session_epoch,
            }),
            workflow: None,
        };
        Client::new(self.endpoint(), auth.workspace, authority, Some(&self.ca)).expect("client")
    }
    fn host_client(&self, auth: &Authority3) -> Client {
        self.client_for(auth, &auth.host, &auth.host_session)
    }
    fn readiness(&mut self) -> Readiness {
        if let Some(forward) = self.forward.as_mut() {
            forward.ensure_alive();
        }
        let endpoint = self.endpoint();
        self.runtime
            .block_on(readiness(
                &endpoint,
                Some(&self.ca),
                Duration::from_secs(10),
            ))
            .unwrap_or_else(|error| {
                panic!("blocked: port-forward refused under NetworkPolicy ({error})")
            })
    }
    fn service_restart_count(&self) -> i64 {
        let pods = self.kubectl.get_json(&[
            "pods",
            "-l",
            "app.kubernetes.io/name=anchorfs,app.kubernetes.io/component=service",
        ]);
        pods["items"]
            .as_array()
            .map(|all| {
                all.iter()
                    .flat_map(|p| {
                        p["status"]["containerStatuses"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                    })
                    .filter_map(|c| c["restartCount"].as_i64())
                    .sum()
            })
            .unwrap_or(0)
    }
    fn create_worker(
        &mut self,
        auth: &Authority3,
        tag: &str,
        node_index: usize,
        worker_index: usize,
        session: &SessionRef,
        workflow: Option<(&str, i64)>,
    ) -> WorkerPod {
        let name = self.pod_name(tag);
        let node = self.node_of(node_index);
        let secret = self.secret_name(worker_index);
        let pod = self.pods.create(&WorkerSpec {
            name: &name,
            node: &node,
            workspace: auth.workspace,
            session,
            credential_secret: &secret,
            workflow,
        });
        let facts = self.pods.facts(&pod);
        assert_eq!(facts.node_name, node, "pod {name} landed on the wrong node");
        let arch = self.arch_of(&node);
        self.evidence.push(
            "pods",
            json!({
                "name": name,
                "tag": tag,
                "node": facts.node_name,
                "arch": arch,
                "uid": facts.uid,
                "image_ids": facts.image_ids,
                "ready_at_unix": unix_now(),
            }),
        );
        pod
    }
    fn delete_pod(&mut self, pod: &WorkerPod) -> DeleteReport {
        let report = self.pods.delete_pod(pod);
        self.evidence.push(
            "deleted_pods",
            json!({
                "name": pod.name,
                "node": pod.node,
                "mount_exit_code": report.mount_exit_code,
                "removed_after_ms": report.removed_after.as_millis() as u64,
            }),
        );
        report
    }
    fn exec_ok(&self, pod: &WorkerPod, script: &str) -> String {
        let output = self.pods.exec(pod, script);
        assert!(
            output.success(),
            "exec on {} failed (code {:?}): {}",
            pod.name,
            output.code,
            output.stderr.trim()
        );
        output.stdout
    }
    fn pod_sha256(&self, pod: &WorkerPod, path: &str) -> String {
        let out = self.exec_ok(pod, &format!("sha256sum {path}"));
        out.split_whitespace().next().unwrap_or("").to_owned()
    }
    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.runtime.block_on(future)
    }
    fn write_file(&self, client: &Client, path: &str, bytes: &[u8]) {
        self.block_on(async {
            let grant = client
                .open_handle(
                    &OpenHandle {
                        path: path.into(),
                        read: true,
                        write: true,
                        create: true,
                        exclusive: true,
                        truncate: false,
                        mode: Some(0o644),
                    },
                    &IdempotencyKey::new(),
                )
                .await
                .expect("open for create");
            for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
                client
                    .write_at(
                        grant.handle_id,
                        (index * CHUNK) as u64,
                        Bytes::copy_from_slice(chunk),
                        &IdempotencyKey::new(),
                    )
                    .await
                    .expect("write_at");
            }
            client.fsync(grant.handle_id).await.expect("fsync");
            client
                .close_handle(grant.handle_id, &IdempotencyKey::new())
                .await
                .expect("close");
        });
    }
    fn read_file(&self, client: &Client, path: &str) -> Vec<u8> {
        self.block_on(async {
            let grant = client
                .open_handle(
                    &OpenHandle {
                        path: path.into(),
                        read: true,
                        write: false,
                        create: false,
                        exclusive: false,
                        truncate: false,
                        mode: None,
                    },
                    &IdempotencyKey::new(),
                )
                .await
                .expect("open for read");
            let size = grant.attributes.size;
            let mut out = Vec::with_capacity(size as usize);
            while (out.len() as u64) < size {
                let want = (size - out.len() as u64).min(CHUNK as u64) as u32;
                let (bytes, _) = client
                    .read_range(grant.handle_id, out.len() as u64, want)
                    .await
                    .expect("read_range");
                assert!(!bytes.is_empty(), "short read of {path}");
                out.extend_from_slice(&bytes);
            }
            client
                .close_handle(grant.handle_id, &IdempotencyKey::new())
                .await
                .expect("close");
            out
        })
    }
    fn stat(&self, client: &Client, path: &str) -> (u64, i64) {
        let attributes = self
            .block_on(client.stat(Some(path)))
            .unwrap_or_else(|e| panic!("stat {path}: {e}"));
        (attributes.size, attributes.generation)
    }
    fn mount_log_has(&self, pod: &WorkerPod, message: &str, reason: Option<&str>) -> bool {
        log_has_event(&self.kubectl.logs(&pod.name, "mount"), message, reason)
    }
    fn wait_for<T>(&self, what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(value) = probe() {
                return value;
            }
            thread::sleep(Duration::from_secs(2));
        }
        panic!("timed out after {}s waiting for {what}", WAIT.as_secs());
    }

    // ---- phase 0 --------------------------------------------------------------------

    fn preflight(&mut self) {
        self.enter("0-preflight");
        let nodes = self.kubectl.get_json(&["nodes"]);
        let items = nodes["items"].as_array().cloned().unwrap_or_default();
        let find = |name: &str| {
            items
                .iter()
                .find(|n| n["metadata"]["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("blocked: node {name} not found"))
        };
        let mut table = Vec::new();
        let mut expected: Vec<(String, &str)> = self
            .cfg
            .worker_nodes
            .iter()
            .take(2)
            .map(|n| (n.clone(), "arm64"))
            .collect();
        expected.push((self.cfg.service_node.clone(), "amd64"));
        for (name, arch) in &expected {
            let node = find(name);
            let ready = node["status"]["conditions"].as_array().is_some_and(|all| {
                all.iter()
                    .any(|c| c["type"] == "Ready" && c["status"] == "True")
            });
            if !ready {
                panic!("blocked: node {name} not Ready");
            }
            let actual = node["metadata"]["labels"]["kubernetes.io/arch"]
                .as_str()
                .unwrap_or("");
            assert_eq!(actual, *arch, "node {name} architecture");
            self.node_arch.push((name.clone(), actual.to_owned()));
            table.push(json!({
                "name": name,
                "arch": actual,
                "kernel": node["status"]["nodeInfo"]["kernelVersion"],
                "kubelet": node["status"]["nodeInfo"]["kubeletVersion"],
            }));
        }
        self.evidence.set("nodes", json!(table));

        let deployment = self.kubectl.get_json(&["deployment", "anchorfs"]);
        assert!(
            deployment["status"]["availableReplicas"]
                .as_i64()
                .unwrap_or(0)
                >= 1,
            "Deployment anchorfs is not available"
        );
        let service_pods = self.kubectl.get_json(&[
            "pods",
            "-l",
            "app.kubernetes.io/name=anchorfs,app.kubernetes.io/component=service",
        ]);
        let on_service_node = service_pods["items"].as_array().is_some_and(|all| {
            all.iter()
                .any(|p| p["spec"]["nodeName"] == self.cfg.service_node.as_str())
        });
        assert!(
            on_service_node,
            "service pod is not on node {}",
            self.cfg.service_node
        );
        self.service_restarts = self.service_restart_count();

        let keys = self.kubectl.secret_keys("anchorfs-tls");
        assert!(
            keys.iter().any(|k| k == "ca.crt"),
            "blocked: G3: secret anchorfs-tls has no ca.crt (keys: {keys:?}); tls.crt of the CA secret is the documented fallback"
        );
        self.evidence
            .phase("0-preflight", "anchorfs_tls_keys", json!(keys));
        self.ca = self.kubectl.secret_value("anchorfs-tls", "ca.crt");

        self.check_cert_manager();

        let stale = self
            .kubectl
            .get_json(&["pods", "-l", "anchorfs.proof/role=worker"]);
        let stale_names: Vec<String> = stale["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p["metadata"]["name"].as_str().map(str::to_owned))
            .collect();
        if !stale_names.is_empty() {
            eprintln!("deleting stale proof pods: {stale_names:?}");
            self.kubectl
                .delete_by_label("pod", "anchorfs.proof/role=worker");
        }

        let service = self.kubectl.get_json(&["service", "anchorfs"]);
        let port = service["spec"]["ports"][0]["port"]
            .as_u64()
            .expect("service port") as u16;
        let mut forward = self.kubectl.port_forward("svc/anchorfs", port);
        if !forward.wait_listening(Duration::from_secs(60)) {
            eprintln!("port-forward stderr: {}", forward.stderr_text());
            panic!("blocked: port-forward refused under NetworkPolicy");
        }
        self.forward = Some(forward);
        let ready = self.readiness();
        assert_eq!(
            ready.status, 200,
            "readyz status; error body {:?}",
            ready.error
        );
        let body = ready.ready.expect("readyz body");
        assert!(body.tls, "readyz reports tls: false");
        assert_eq!(body.durability_profile, "durable");
        self.evidence
            .set("readyz", serde_json::to_value(&body).unwrap_or(Value::Null));
        self.api = Some(Api::new(
            Arc::clone(&self.runtime),
            &format!(
                "https://localhost:{}",
                self.forward.as_ref().expect("forward").local_port
            ),
            &self.ca,
        ));
    }

    fn check_cert_manager(&mut self) {
        let pods = self.kubectl.get_json(&["pods", "-n", "cert-manager"]);
        let mut images = BTreeSet::new();
        let mut states = Vec::new();
        for pod in pods["items"].as_array().cloned().unwrap_or_default() {
            for container in pod["spec"]["containers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                if let Some(image) = container["image"].as_str() {
                    images.insert(image.to_owned());
                }
            }
            states.push(format!(
                "{}: {}",
                pod["metadata"]["name"].as_str().unwrap_or("?"),
                pod["status"]["phase"].as_str().unwrap_or("?")
            ));
        }
        self.evidence
            .phase("0-preflight", "cert_manager_images", json!(images));
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let certs = self.kubectl.get_json(&["certificates"]);
            let not_ready: Vec<String> = certs["items"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter(|c| {
                    !c["status"]["conditions"].as_array().is_some_and(|all| {
                        all.iter()
                            .any(|x| x["type"] == "Ready" && x["status"] == "True")
                    })
                })
                .map(|c| c["metadata"]["name"].as_str().unwrap_or("?").to_owned())
                .collect();
            if not_ready.is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "blocked: certificates not Ready within 120s: {not_ready:?}; cert-manager pods {states:?} (not an anchorfs defect)"
                );
            }
            thread::sleep(Duration::from_secs(5));
        }
    }

    // ---- phase 1 --------------------------------------------------------------------

    fn authority_setup(&mut self) -> Authority3 {
        self.enter("1-authority");
        let api = self.api.as_ref().expect("api");
        let admin = Bearer::from_bootstrap_json(
            &self
                .kubectl
                .secret_value("anchorfs-bootstrap-admin", "credential"),
        );
        let workspace = api.create_workspace(&admin, WORKER_UID, WORKER_UID);
        let host = api.create_credential(&admin, workspace, "host", WORKER_UID, WORKER_UID);
        let workers: Vec<Bearer> = (0..3)
            .map(|_| api.create_credential(&admin, workspace, "worker", WORKER_UID, WORKER_UID))
            .collect();
        let host_session = api.create_session(&host, workspace, host.credential_id, None);
        let sessions: Vec<SessionRef> = workers
            .iter()
            .map(|w| api.create_session(&host, workspace, w.credential_id, None))
            .collect();
        let label = [("anchorfs.proof/run", self.run.as_str())];
        for (index, worker) in workers.iter().enumerate() {
            self.kubectl
                .create_secret(&self.secret_name(index), "bearer", worker.expose(), &label);
        }
        self.evidence
            .phase("1-authority", "workspace_id", json!(workspace.to_string()));
        Authority3 {
            admin,
            host,
            host_session,
            workspace,
            workers,
            sessions,
        }
    }

    // ---- phase 2 --------------------------------------------------------------------

    /// Returns the live pod A and the digests of `harness.txt` and `worker-a.bin`.
    fn case1_harness_to_worker_a(
        &mut self,
        auth: &Authority3,
        client: &Client,
    ) -> (WorkerPod, Digests) {
        self.enter("2-case1-harness-writes-worker-reads");
        self.block_on(client.create_directory("proof", 0o755, &IdempotencyKey::new()))
            .expect("mkdir proof");
        let harness_bytes: Vec<u8> = (0..3072u32).map(|i| (i * 7 + 13) as u8).collect();
        self.write_file(client, "proof/harness.txt", &harness_bytes);
        let harness = sha256_hex(&harness_bytes);

        let pod = self.create_worker(auth, "a", 0, 0, &auth.sessions[0], None);
        let seen = self.pod_sha256(&pod, &format!("{WORKSPACE_PATH}/proof/harness.txt"));
        assert_eq!(seen, harness, "worker A digest of harness.txt");

        self.exec_ok(
            &pod,
            &format!(
                "dd if=/dev/urandom of={WORKSPACE_PATH}/proof/worker-a.bin bs=1M count=3 conv=fsync status=none"
            ),
        );
        let worker = self.pod_sha256(&pod, &format!("{WORKSPACE_PATH}/proof/worker-a.bin"));
        let bytes = self.read_file(client, "proof/worker-a.bin");
        assert_eq!(sha256_hex(&bytes), worker, "API read of worker-a.bin");
        let (size, _) = self.stat(client, "proof/worker-a.bin");
        assert_eq!(size, 3 << 20, "stat size of worker-a.bin");
        self.evidence.phase(
            "2-case1",
            "digests",
            json!({"harness.txt": harness, "worker-a.bin": worker}),
        );
        (pod, Digests { harness, worker })
    }

    // ---- phase 3 --------------------------------------------------------------------

    fn case2_replacement_on_other_node(
        &mut self,
        auth: &Authority3,
        pod_a: WorkerPod,
        digests: &Digests,
    ) {
        self.enter("3-case2-replacement-on-other-node");
        self.delete_pod(&pod_a);
        let pod_b = self.create_worker(auth, "b", 1, 1, &auth.sessions[1], None);
        let facts_b = self.pods.facts(&pod_b);
        assert_ne!(
            facts_b.node_name, pod_a.node,
            "pod B must run on a different node"
        );
        self.assert_files(&pod_b, digests);
        let listing = self.exec_ok(&pod_b, &format!("ls {WORKSPACE_PATH}/proof"));
        let mut names: Vec<&str> = listing.split_whitespace().collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["harness.txt", "worker-a.bin"],
            "proof directory listing"
        );
        for id in &facts_b.image_ids {
            assert!(
                id.contains("anchorfs/mount@sha256:"),
                "image ID {id} does not reference anchorfs/mount@sha256:<digest>"
            );
        }
        let sidecar = self.kubectl.logs(&pod_b.name, "mount");
        self.evidence.phase(
            "3-case2",
            "t16",
            json!({
                "pod_a_node": pod_a.node,
                "pod_b_node": facts_b.node_name,
                "pod_b_arch": self.arch_of(&facts_b.node_name),
                "pod_b_uid": facts_b.uid,
                "pod_b_image_ids": facts_b.image_ids,
                "pod_b_sidecar_log_lines": sidecar.lines().count(),
                "copy_steps_executed": 0,
            }),
        );
        self.delete_pod(&pod_b);
    }

    fn assert_files(&self, pod: &WorkerPod, digests: &Digests) {
        for (file, expected) in [
            ("harness.txt", &digests.harness),
            ("worker-a.bin", &digests.worker),
        ] {
            let seen = self.pod_sha256(pod, &format!("{WORKSPACE_PATH}/proof/{file}"));
            assert_eq!(&seen, expected, "digest of {file} on pod {}", pod.name);
        }
    }

    // ---- phase 4 --------------------------------------------------------------------

    fn case3_idle_then_attach(&mut self, auth: &Authority3, digests: &Digests) -> WorkerPod {
        self.enter("4-case3-idle-then-attach");
        let idle_started = Instant::now();
        thread::sleep(Duration::from_secs(90));
        let restarts = self.service_restart_count();
        assert_eq!(
            restarts, self.service_restarts,
            "service restart count changed while no worker pod existed"
        );
        let ready = self.readiness();
        assert_eq!(ready.status, 200, "readyz after the idle period");
        self.evidence.phase(
            "4-case3",
            "idle",
            json!({"seconds": idle_started.elapsed().as_secs(), "service_restarts": restarts}),
        );
        let pod_c = self.create_worker(auth, "c", 0, 2, &auth.sessions[2], None);
        self.assert_files(&pod_c, digests);
        pod_c
    }

    // ---- phase 5 --------------------------------------------------------------------

    /// Revoke first (D19), then lease and bound successor. Returns the live successor pod D.
    fn case5_6_fencing_then_successor(
        &mut self,
        auth: &Authority3,
        client: &Client,
        pod_c: WorkerPod,
    ) -> (WorkerPod, Lease) {
        self.enter("5-case5-6-fencing-then-successor");
        let fence_log = "proof/fence.log";
        let fence_path = format!("{WORKSPACE_PATH}/{fence_log}");

        // (a) a worker-side append loop, confirmed to grow through the API.
        self.pods.start_tick_loop(&pod_c, &fence_path);
        let first = self.wait_for("fence.log to appear", || {
            self.block_on(client.stat(Some(fence_log)))
                .ok()
                .map(|a| a.size)
        });
        let second = self.wait_for("fence.log to grow", || {
            let size = self.block_on(client.stat(Some(fence_log))).ok()?.size;
            (size > first).then_some(size)
        });
        self.evidence
            .phase("5-case5-6", "growth_samples", json!([first, second]));

        // (b) revoke worker-3 first, then prove the mount latches with ESTALE.
        let api = self.api.as_ref().expect("api");
        api.revoke_credential(&auth.admin, auth.workers[2].credential_id);
        self.wait_for("the \"mount fenced\" event (unauthenticated)", || {
            self.mount_log_has(&pod_c, "mount fenced", Some("unauthenticated"))
                .then_some(())
        });
        let (size_at_revocation, generation_at_revocation) = self.stat(client, fence_log);
        let late = self
            .pods
            .exec(&pod_c, &format!("echo late >> {fence_path}"));
        assert!(
            !late.success() && !late.timed_out,
            "late write after revocation must fail, got exit {:?}",
            late.code
        );
        let late_text = late.stderr.trim().to_owned();
        assert!(
            late_text.contains("Stale file handle"),
            "late write must fail with ESTALE (Stale file handle), got: {late_text}"
        );
        assert!(
            !late_text.contains("busy"),
            "EBUSY here would mean lease-busy, which has not been introduced yet: {late_text}"
        );
        self.evidence.phase(
            "5-case5-6",
            "revocation",
            json!({
                "size_at_revocation": size_at_revocation,
                "generation_at_revocation": generation_at_revocation,
                "late_write_exit_code": late.code,
                "late_write_error_estale": late_text,
            }),
        );

        // (c) lease, bound session for worker-2's credential, successor pod D.
        let api = self.api.as_ref().expect("api");
        let lease: Lease = api.acquire_lease(&auth.host, &auth.host_session, auth.workspace);
        let bound = api.create_session(
            &auth.host,
            auth.workspace,
            auth.workers[1].credential_id,
            Some(&lease.token),
        );
        let workflow_secret = format!("anchorfs-workflow-{}", &self.run[..8]);
        self.kubectl.create_secret(
            &workflow_secret,
            "token",
            &lease.token,
            &[("anchorfs.proof/run", self.run.as_str())],
        );
        let pod_d = self.create_worker(
            auth,
            "d",
            1,
            1,
            &bound,
            Some((workflow_secret.as_str(), lease.epoch)),
        );
        self.exec_ok(
            &pod_d,
            &format!(
                "echo successor | dd of={WORKSPACE_PATH}/proof/successor.txt conv=fsync status=none"
            ),
        );
        let successor = self.read_file(client, "proof/successor.txt");
        assert_eq!(successor, b"successor\n", "successor.txt through the API");
        // A second observation of the revoked mount after the lease exists; recorded, not
        // asserted, as a distinct phenomenon from the revocation latch above.
        let after_lease = self
            .pods
            .exec(&pod_c, &format!("echo again >> {fence_path}"));
        self.evidence.observe(
            "ordinary_mount_write_after_lease",
            json!({"exit_code": after_lease.code, "stderr": after_lease.stderr.trim()}),
        );

        // (d) negatives with the revoked worker-3 bearer; the host and worker-2 keep working.
        let revoked = &auth.workers[2];
        let api = self.api.as_ref().expect("api");
        let negatives = [
            (
                "createSession",
                api.create_session_raw(revoked, auth.workspace, revoked.credential_id)
                    .status,
            ),
            (
                "renewSession",
                api.renew_session_raw(revoked, &auth.sessions[2], auth.workspace)
                    .status,
            ),
            (
                "rebindSession",
                api.rebind_session_raw(revoked, &auth.sessions[2], auth.workspace)
                    .status,
            ),
            (
                "acquireWorkflowLease",
                api.acquire_lease_raw(revoked, &auth.sessions[2], auth.workspace)
                    .status,
            ),
        ];
        for (operation, status) in negatives {
            assert!(
                matches!(status, 401 | 403),
                "{operation} with the revoked bearer returned {status}, expected 401/403"
            );
        }
        self.evidence.phase(
            "5-case5-6",
            "revoked_bearer_statuses",
            json!(
                negatives
                    .iter()
                    .map(|(o, s)| json!({"operation": o, "status": s}))
                    .collect::<Vec<_>>()
            ),
        );
        self.stat(client, "proof/successor.txt");
        self.exec_ok(
            &pod_d,
            &format!("echo more | dd of={WORKSPACE_PATH}/proof/successor.txt oflag=append conv=notrunc,fsync status=none"),
        );
        let successor = self.read_file(client, "proof/successor.txt");
        assert_eq!(successor, b"successor\nmore\n", "worker-2 keeps working");

        // (e) the lease is released only after pod D (bound to it) has finished phase 6:
        // a bound session fails closed after release/takeover (Stage 2a), so releasing
        // here would fence pod D's later writes. Pod C's loop is killed by delete_pod's
        // quiesce step.
        self.delete_pod(&pod_c);

        // (f) no byte was published after revocation.
        let (final_size, final_generation) = self.stat(client, fence_log);
        assert_eq!(
            (final_size, final_generation),
            (size_at_revocation, generation_at_revocation),
            "fence.log changed after revocation"
        );
        self.evidence.phase(
            "5-case5-6",
            "final",
            json!({"size": final_size, "generation": final_generation}),
        );
        (pod_d, lease)
    }

    // ---- phase 6 --------------------------------------------------------------------

    fn case7_ephemeral_root(
        &mut self,
        auth: &Authority3,
        client: &Client,
        pod_d: WorkerPod,
        lease: Lease,
    ) {
        self.enter("6-case7-ephemeral-root");
        self.exec_ok(&pod_d, "echo ephemeral > /tmp/outside.txt");
        self.exec_ok(
            &pod_d,
            &format!("echo durable | dd of={WORKSPACE_PATH}/proof/inside.txt conv=fsync status=none; sync"),
        );
        self.delete_pod(&pod_d);
        // Pod D (bound session) is gone; release the lease so the ordinary pod E can mutate.
        let api = self.api.as_ref().expect("api");
        api.release_lease(&auth.host, &auth.host_session, auth.workspace, &lease);
        self.evidence
            .phase("6-case7", "lease_released", json!({"epoch": lease.epoch}));
        let fresh = api.create_session(
            &auth.host,
            auth.workspace,
            auth.workers[0].credential_id,
            None,
        );
        let pod_e = self.create_worker(auth, "e", 0, 0, &fresh, None);
        let inside = self.exec_ok(&pod_e, &format!("cat {WORKSPACE_PATH}/proof/inside.txt"));
        assert_eq!(inside.trim(), "durable", "inside.txt content on pod E");
        let outside = self.pods.exec(&pod_e, "test ! -e /tmp/outside.txt");
        assert!(
            outside.success(),
            "/tmp/outside.txt must be absent on pod E"
        );
        let _ = self.read_file(client, "proof/inside.txt");
        self.evidence.phase(
            "6-case7",
            "result",
            json!({"inside_txt": "durable", "outside_txt_absent": true}),
        );
        self.delete_pod(&pod_e);
    }

    // ---- phase 7 --------------------------------------------------------------------

    fn teardown(&mut self) {
        self.enter("7-teardown");
        self.pods.cleanup();
        let elapsed = self.started.elapsed();
        self.evidence
            .observe("total_duration_seconds", json!(elapsed.as_secs()));
        self.evidence.phase(
            "7-teardown",
            "released",
            json!("pods and per-run Secrets deleted; release and workspace kept"),
        );
    }
}

struct Digests {
    harness: String,
    worker: String,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Minimal SHA-256 (FIPS 180-4) so the test needs no new dependency.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

#[test]
fn sha256_matches_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
