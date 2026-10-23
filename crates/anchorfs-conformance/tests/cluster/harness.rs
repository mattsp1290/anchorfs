//! Plumbing shared by the proof phases: the phase-0 context, typed-client file I/O, readiness,
//! polling, pod creation and deletion with evidence, and the image-digest assertion.
use crate::{
    api::Api,
    cases::Proof,
    evidence,
    journey::Principals,
    kube::{Kubectl, MAX_WAIT, log_has_event},
    pods::{ContainerImage, DeleteReport, Pods, WorkerPod, WorkerSpec},
};
use anchorfs_client::{
    Client,
    auth::{Authority, SessionAuthority},
    readiness::{Readiness, readiness as probe_readiness},
    retry::IdempotencyKey,
    wire::OpenHandle,
};
use bytes::Bytes;
use serde_json::json;
use std::{collections::BTreeMap, thread, time::Duration, time::Instant};
use uuid::Uuid;

const CHUNK: usize = 1 << 20;

/// Everything phase 0 establishes; later phases cannot run without it.
pub struct Connected {
    pub api: Api,
    pub pods: Pods,
    pub node_arch: BTreeMap<String, String>,
    pub service_restarts: i64,
}

impl Connected {
    pub fn arch_of(&self, node: &str) -> String {
        self.node_arch.get(node).cloned().unwrap_or_default()
    }
    pub fn client_for(&self, workspace: Uuid, authority: Authority) -> Client {
        self.api.client(workspace, authority)
    }
    pub fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.api.block_on(future)
    }
    pub fn readiness(&self) -> Readiness {
        let endpoint = self.api.endpoint();
        self.block_on(probe_readiness(
            &endpoint,
            Some(self.api.ca()),
            Duration::from_secs(10),
        ))
        .unwrap_or_else(|error| {
            panic!(
                "blocked: readiness probe failed ({error}); port-forward stderr: {}",
                self.api.forward_stderr().trim()
            )
        })
    }
    pub fn write_file(&self, client: &Client, path: &str, bytes: &[u8]) {
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
    pub fn read_file(&self, client: &Client, path: &str) -> Vec<u8> {
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
    pub fn stat(&self, client: &Client, path: &str) -> (u64, i64) {
        let attributes = self
            .block_on(client.stat(Some(path)))
            .unwrap_or_else(|e| panic!("stat {path}: {e}"));
        (attributes.size, attributes.generation)
    }
    pub fn exec_ok(&self, pod: &WorkerPod, script: &str) -> String {
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
    pub fn pod_sha256(&self, pod: &WorkerPod, path: &str) -> String {
        let out = self.exec_ok(pod, &format!("sha256sum {path}"));
        out.split_whitespace().next().unwrap_or("").to_owned()
    }
    pub fn mount_log_has(&self, pod: &WorkerPod, message: &str, reason: Option<&str>) -> bool {
        log_has_event(&self.pods.kubectl.logs(&pod.name, "mount"), message, reason)
    }
}

pub fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + MAX_WAIT;
    while Instant::now() < deadline {
        if let Some(value) = probe() {
            return value;
        }
        thread::sleep(Duration::from_secs(2));
    }
    panic!("timed out after {}s waiting for {what}", MAX_WAIT.as_secs());
}

pub fn service_restart_count(kubectl: &Kubectl) -> i64 {
    let pods = kubectl.get_json(&[
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

/// Which pod to create: its tag, the worker node index and the worker credential index.
#[derive(Clone, Copy)]
pub struct Slot<'a> {
    pub tag: &'a str,
    pub node_index: usize,
    pub worker_index: usize,
}

impl Proof {
    pub fn node_of(&self, index: usize) -> String {
        self.cfg.worker_nodes[index].clone()
    }
    fn pod_name(&self, tag: &str) -> String {
        format!("anchorfs-proof-{tag}-{}", &self.run[..8])
    }
    pub fn secret_name(index: usize) -> String {
        format!("anchorfs-worker-{}-credential", index + 1)
    }
    pub fn create_worker(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        slot: Slot<'_>,
        session: &SessionAuthority,
        workflow: Option<(&str, i64)>,
    ) -> WorkerPod {
        let Slot {
            tag,
            node_index,
            worker_index,
        } = slot;
        let name = self.pod_name(tag);
        let node = self.node_of(node_index);
        let secret = Self::secret_name(worker_index);
        let pod = conn.pods.create(&WorkerSpec {
            name: &name,
            node: &node,
            workspace: auth.workspace,
            session,
            credential_secret: &secret,
            workflow,
        });
        let facts = conn.pods.facts(&pod);
        assert_eq!(facts.node_name, node, "pod {name} landed on the wrong node");
        let image_ids: Vec<&str> = facts
            .containers
            .iter()
            .map(|c| c.image_id.as_str())
            .collect();
        self.evidence.push(
            "pods",
            json!({
                "name": name,
                "tag": tag,
                "node": facts.node_name,
                "arch": conn.arch_of(&node),
                "uid": facts.uid,
                "image_ids": image_ids,
                "ready_at_unix": evidence::now(),
            }),
        );
        pod
    }
    pub fn delete_pod(&mut self, conn: &Connected, pod: &WorkerPod) -> DeleteReport {
        let report = conn.pods.delete_pod(pod);
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
}

/// The runtime-reported image ID must carry the same `sha256:` digest as the digest-pinned
/// reference the pod spec was rendered with (the published image), not merely some mount image.
pub fn assert_image_matches_spec(container: &ContainerImage) {
    let spec = digest_of(&container.image).unwrap_or_else(|| {
        panic!(
            "container {}: spec image {} is not pinned by a sha256 digest",
            container.name, container.image
        )
    });
    let reported = digest_of(&container.image_id).unwrap_or_else(|| {
        panic!(
            "container {}: image ID {:?} carries no sha256 digest",
            container.name, container.image_id
        )
    });
    assert_eq!(
        reported, spec,
        "container {}: image ID {} does not match the pod spec image {}",
        container.name, container.image_id, container.image
    );
    if container.name == "mount" {
        assert!(
            container.image.contains("anchorfs/mount@sha256:"),
            "mount container image {} does not reference anchorfs/mount@sha256:<digest>",
            container.image
        );
    }
}

/// The `sha256:<hex>` suffix of an `<repo>@sha256:<hex>` reference.
fn digest_of(reference: &str) -> Option<&str> {
    let (_, digest) = reference.rsplit_once('@')?;
    digest.strip_prefix("sha256:").map(|_| digest)
}

#[test]
fn digest_of_reads_the_pinned_digest_only() {
    assert_eq!(digest_of("r/anchorfs/mount@sha256:ab"), Some("sha256:ab"));
    assert_eq!(digest_of("r/anchorfs/mount:latest"), None);
    assert_eq!(digest_of("r/m@md5:ab"), None);
}
