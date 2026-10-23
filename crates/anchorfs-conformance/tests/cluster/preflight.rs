//! Phase 0: nodes, release, TLS CA, certificates, port-forward and readiness.
use crate::{
    api::Api,
    cases::Proof,
    harness::{Connected, service_restart_count},
    pods::Pods,
};
use anchorfs_client::readiness::Readiness;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

impl Proof {
    pub fn preflight(&mut self) -> Connected {
        self.enter("0-preflight");
        // Read inside the panic guard so a missing ConfigMap is a failed phase in the evidence.
        let template = self
            .kubectl
            .get_json(&["configmap", "anchorfs-worker-template"])["data"]["pod.yaml"]
            .as_str()
            .expect("ConfigMap anchorfs-worker-template has no pod.yaml")
            .to_owned();
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
        let mut node_arch = BTreeMap::new();
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
            node_arch.insert(name.clone(), actual.to_owned());
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
        let service_restarts = service_restart_count(&self.kubectl);

        let keys = self.kubectl.secret_keys("anchorfs-tls");
        assert!(
            keys.iter().any(|k| k == "ca.crt"),
            "blocked: G3: secret anchorfs-tls has no ca.crt (keys: {keys:?}); tls.crt of the CA secret is the documented fallback"
        );
        self.evidence
            .phase("0-preflight", "anchorfs_tls_keys", json!(keys));
        let ca = self.kubectl.secret_value("anchorfs-tls", "ca.crt");

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
        let api = Api::new(Arc::clone(&self.runtime), Arc::new(Mutex::new(forward)), ca);
        let conn = Connected {
            api,
            pods: Pods::new(&self.kubectl, &self.run, template),
            node_arch,
            service_restarts,
        };
        let body = match conn.readiness() {
            Readiness::Ready(body) => body,
            Readiness::NotReady { status, error } => {
                panic!("readyz status {status}; error body {error:?}")
            }
        };
        assert!(body.tls, "readyz reports tls: false");
        assert_eq!(body.durability_profile, "durable");
        self.evidence
            .set("readyz", serde_json::to_value(&body).unwrap_or(Value::Null));
        conn
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
}
