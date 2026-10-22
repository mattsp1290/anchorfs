//! Sanitized evidence document. Only identifiers, digests of public files, node facts, exit
//! codes and timestamps are recorded; bearers, lease tokens and Secret values never reach it.
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Evidence {
    path: PathBuf,
    root: Value,
}
impl Evidence {
    pub fn new(path: &str, run: &str, kubectl_version: &str) -> Self {
        let mut evidence = Self {
            path: PathBuf::from(path),
            root: json!({
                "status": "running",
                "run_id": run,
                "started_at_unix": now(),
                "kubectl": kubectl_version,
                "failed_phase": null,
                "nodes": [],
                "pods": [],
                "deleted_pods": [],
                "phases": {},
                "readyz": null,
                "observations": {},
            }),
        };
        evidence.write();
        evidence
    }
    /// Records one value under a phase heading and persists the partial document.
    pub fn phase(&mut self, phase: &str, key: &str, value: Value) {
        self.root["phases"][phase][key] = value;
        self.write();
    }
    pub fn set(&mut self, key: &str, value: Value) {
        self.root[key] = value;
        self.write();
    }
    pub fn push(&mut self, key: &str, value: Value) {
        if let Some(list) = self.root[key].as_array_mut() {
            list.push(value);
        }
        self.write();
    }
    pub fn observe(&mut self, key: &str, value: Value) {
        self.root["observations"][key] = value;
        self.write();
    }
    pub fn finish(&mut self, status: &str, failed_phase: Option<&str>) {
        self.root["status"] = json!(status);
        self.root["failed_phase"] = json!(failed_phase);
        self.root["finished_at_unix"] = json!(now());
        self.write();
    }
    pub fn write(&mut self) {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(&self.root).expect("evidence serializes");
        // Write-then-rename keeps a failed run's partial file parseable.
        let temp = self.path.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, &self.path);
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
