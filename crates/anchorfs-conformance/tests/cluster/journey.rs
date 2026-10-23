//! Phases 1-4: authority setup, then cases 1 (harness writes, worker reads), 2 (replacement on
//! the other node) and 3 (idle, then attach).
use crate::{
    api::Credential,
    cases::Proof,
    harness::{Connected, Slot, assert_image_matches_spec},
    pods::{WORKSPACE_PATH, WorkerPod},
};
use anchorfs_client::{
    Client,
    auth::{Authority, SessionAuthority},
    readiness::Readiness,
    retry::IdempotencyKey,
};
use anchorfs_core::sha256;
use serde_json::json;
use std::{thread, time::Duration, time::Instant};
use uuid::Uuid;

const WORKER_UID: u32 = 10001;

/// The credentials and sessions the proof acts as.
pub struct Principals {
    pub admin: Credential,
    pub host: Credential,
    pub host_session: SessionAuthority,
    pub workspace: Uuid,
    pub workers: Vec<Credential>,
    pub sessions: Vec<SessionAuthority>,
}
impl Principals {
    pub fn admin_authority(&self) -> Authority {
        self.admin.authority(None)
    }
    /// The host credential with its unbound session.
    pub fn host_authority(&self) -> Authority {
        self.host.authority(Some(&self.host_session))
    }
}

pub struct Digests {
    harness: String,
    worker: String,
}

impl Proof {
    pub fn authority_setup(&mut self, conn: &Connected) -> Principals {
        self.enter("1-authority");
        let api = &conn.api;
        let admin = Credential::from_bootstrap_json(
            &self
                .kubectl
                .secret_value("anchorfs-bootstrap-admin", "credential"),
        );
        let admin_authority = admin.authority(None);
        let workspace = api.create_workspace(&admin_authority, WORKER_UID, WORKER_UID);
        let host =
            api.create_credential(&admin_authority, workspace, "host", WORKER_UID, WORKER_UID);
        let workers: Vec<Credential> = (0..3)
            .map(|_| {
                api.create_credential(
                    &admin_authority,
                    workspace,
                    "worker",
                    WORKER_UID,
                    WORKER_UID,
                )
            })
            .collect();
        let host_unbound = host.authority(None);
        let host_session = api.create_session(&host_unbound, workspace, host.id());
        let sessions: Vec<SessionAuthority> = workers
            .iter()
            .map(|w| api.create_session(&host_unbound, workspace, w.id()))
            .collect();
        let label = [("anchorfs.proof/run", self.run.as_str())];
        for (index, worker) in workers.iter().enumerate() {
            self.kubectl.create_secret(
                &Self::secret_name(index),
                "bearer",
                worker.composed(),
                &label,
            );
        }
        self.evidence
            .phase("1-authority", "workspace_id", json!(workspace.to_string()));
        Principals {
            admin,
            host,
            host_session,
            workspace,
            workers,
            sessions,
        }
    }

    /// Returns the live pod A and the digests of `harness.txt` and `worker-a.bin`.
    pub fn case1_harness_to_worker_a(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        client: &Client,
    ) -> (WorkerPod, Digests) {
        self.enter("2-case1-harness-writes-worker-reads");
        conn.block_on(client.create_directory("proof", 0o755, &IdempotencyKey::new()))
            .expect("mkdir proof");
        let harness_bytes: Vec<u8> = (0..3072u32).map(|i| (i * 7 + 13) as u8).collect();
        conn.write_file(client, "proof/harness.txt", &harness_bytes);
        let harness = sha256(&harness_bytes).to_string();

        let pod = self.create_worker(
            conn,
            auth,
            Slot {
                tag: "a",
                node_index: 0,
                worker_index: 0,
            },
            &auth.sessions[0],
            None,
        );
        let seen = conn.pod_sha256(&pod, &format!("{WORKSPACE_PATH}/proof/harness.txt"));
        assert_eq!(seen, harness, "worker A digest of harness.txt");

        conn.exec_ok(
            &pod,
            &format!(
                "dd if=/dev/urandom of={WORKSPACE_PATH}/proof/worker-a.bin bs=1M count=3 conv=fsync status=none"
            ),
        );
        let worker = conn.pod_sha256(&pod, &format!("{WORKSPACE_PATH}/proof/worker-a.bin"));
        let bytes = conn.read_file(client, "proof/worker-a.bin");
        assert_eq!(
            sha256(&bytes).to_string(),
            worker,
            "API read of worker-a.bin"
        );
        let (size, _) = conn.stat(client, "proof/worker-a.bin");
        assert_eq!(size, 3 << 20, "stat size of worker-a.bin");
        self.evidence.phase(
            "2-case1",
            "digests",
            json!({"harness.txt": harness, "worker-a.bin": worker}),
        );
        (pod, Digests { harness, worker })
    }

    pub fn case2_replacement_on_other_node(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        pod_a: WorkerPod,
        digests: &Digests,
    ) {
        self.enter("3-case2-replacement-on-other-node");
        self.delete_pod(conn, &pod_a);
        let pod_b = self.create_worker(
            conn,
            auth,
            Slot {
                tag: "b",
                node_index: 1,
                worker_index: 1,
            },
            &auth.sessions[1],
            None,
        );
        let facts_b = conn.pods.facts(&pod_b);
        assert_ne!(
            facts_b.node_name, pod_a.node,
            "pod B must run on a different node"
        );
        assert_files(conn, &pod_b, digests);
        let listing = conn.exec_ok(&pod_b, &format!("ls {WORKSPACE_PATH}/proof"));
        let mut names: Vec<&str> = listing.split_whitespace().collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["harness.txt", "worker-a.bin"],
            "proof directory listing"
        );
        assert!(
            facts_b.containers.len() >= 2,
            "pod B reports {} containers, expected mount and worker",
            facts_b.containers.len()
        );
        for container in &facts_b.containers {
            assert_image_matches_spec(container);
        }
        let image_ids: Vec<&str> = facts_b
            .containers
            .iter()
            .map(|c| c.image_id.as_str())
            .collect();
        let sidecar = conn.pods.kubectl.logs(&pod_b.name, "mount");
        self.evidence.phase(
            "3-case2",
            "t16",
            json!({
                "pod_a_node": pod_a.node,
                "pod_b_node": facts_b.node_name,
                "pod_b_arch": conn.arch_of(&facts_b.node_name),
                "pod_b_uid": facts_b.uid,
                "pod_b_image_ids": image_ids,
                "pod_b_sidecar_log_lines": sidecar.lines().count(),
                "copy_steps_executed": 0,
            }),
        );
        self.delete_pod(conn, &pod_b);
    }

    pub fn case3_idle_then_attach(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        digests: &Digests,
    ) -> WorkerPod {
        self.enter("4-case3-idle-then-attach");
        let idle_started = Instant::now();
        thread::sleep(Duration::from_secs(90));
        let restarts = crate::harness::service_restart_count(&self.kubectl);
        assert_eq!(
            restarts, conn.service_restarts,
            "service restart count changed while no worker pod existed"
        );
        assert!(
            matches!(conn.readiness(), Readiness::Ready(_)),
            "readyz after the idle period"
        );
        self.evidence.phase(
            "4-case3",
            "idle",
            json!({"seconds": idle_started.elapsed().as_secs(), "service_restarts": restarts}),
        );
        let pod_c = self.create_worker(
            conn,
            auth,
            Slot {
                tag: "c",
                node_index: 0,
                worker_index: 2,
            },
            &auth.sessions[2],
            None,
        );
        assert_files(conn, &pod_c, digests);
        pod_c
    }
}

fn assert_files(conn: &Connected, pod: &WorkerPod, digests: &Digests) {
    for (file, expected) in [
        ("harness.txt", &digests.harness),
        ("worker-a.bin", &digests.worker),
    ] {
        let seen = conn.pod_sha256(pod, &format!("{WORKSPACE_PATH}/proof/{file}"));
        assert_eq!(&seen, expected, "digest of {file} on pod {}", pod.name);
    }
}
