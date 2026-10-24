//! Two-node cluster proof against an installed AnchorFS release (plan 03 section 2). Owner-run
//! only: `ANCHORFS_CLUSTER_KUBECONFIG=<kubeconfig> cargo test -p anchorfs-conformance --test
//! cluster --locked -- --ignored --test-threads=1 --nocapture`. Other variables:
//! `ANCHORFS_CLUSTER_KUBECTL` (default `k3s kubectl`), `ANCHORFS_CLUSTER_NAMESPACE`,
//! `ANCHORFS_CLUSTER_WORKER_NODES`, `ANCHORFS_CLUSTER_SERVICE_NODE`,
//! `ANCHORFS_CLUSTER_EVIDENCE`. See `docs/ops/cluster-proof.md`.
#[path = "cluster/api.rs"]
mod api;
#[path = "cluster/cases.rs"]
mod cases;
#[path = "cluster/evidence.rs"]
mod evidence;
#[path = "cluster/fencing.rs"]
mod fencing;
#[path = "cluster/harness.rs"]
mod harness;
#[path = "cluster/journey.rs"]
mod journey;
#[path = "cluster/kube.rs"]
mod kube;
#[path = "cluster/pods.rs"]
mod pods;
#[path = "cluster/preflight.rs"]
mod preflight;

#[test]
#[ignore = "owner-run against a live cluster; see docs/ops/cluster-proof.md"]
fn two_node_cluster_proof() {
    let Ok(kubeconfig) = std::env::var("ANCHORFS_CLUSTER_KUBECONFIG") else {
        panic!("blocked: ANCHORFS_CLUSTER_KUBECONFIG unset");
    };
    cases::run(cases::Config::from_env(kubeconfig));
}
