//! Driver of the two-node cluster proof (plan 03 section 2). Each phase records evidence; any
//! failure records the failing phase, deletes the run's pods and Secrets best-effort and
//! re-raises the original panic. Phases live in `preflight`, `journey` and `fencing`.
use crate::{evidence::Evidence, kube::Kubectl, pods::cleanup_run};
use serde_json::json;
use std::{
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

const TOTAL_BUDGET: Duration = Duration::from_secs(25 * 60);

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

pub struct Proof {
    pub cfg: Config,
    pub kubectl: Kubectl,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub run: String,
    pub started: Instant,
    pub phase: &'static str,
    pub evidence: Evidence,
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
    let mut proof = Proof {
        cfg,
        kubectl,
        runtime,
        run,
        started: Instant::now(),
        phase: "start",
        evidence,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| proof.execute()));
    match outcome {
        Ok(()) => proof.evidence.finish("passed", None),
        Err(panic) => {
            let phase = proof.phase;
            proof.evidence.finish("failed", Some(phase));
            eprintln!("cluster proof failed in phase: {phase}");
            cleanup_run(&proof.kubectl, &proof.run);
            resume_unwind(panic);
        }
    }
}

impl Proof {
    pub fn enter(&mut self, phase: &'static str) {
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
        let conn = self.preflight();
        let auth = self.authority_setup(&conn);
        let client = conn.client_for(auth.workspace, auth.host_authority());
        let (pod_a, digests) = self.case1_harness_to_worker_a(&conn, &auth, &client);
        self.case2_replacement_on_other_node(&conn, &auth, pod_a, &digests);
        let pod_c = self.case3_idle_then_attach(&conn, &auth, &digests);
        let (pod_d, lease) = self.case5_6_fencing_then_successor(&conn, &auth, &client, pod_c);
        self.case7_ephemeral_root(&conn, &auth, &client, pod_d, lease);
        self.teardown();
    }

    fn teardown(&mut self) {
        self.enter("7-teardown");
        cleanup_run(&self.kubectl, &self.run);
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
