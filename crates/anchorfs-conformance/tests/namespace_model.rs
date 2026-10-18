//! Sequential router/model comparison. Concurrency evidence lives in namespace_tx.
mod common;
#[path = "namespace_model/model.rs"]
mod model;
use common::fixture::*;
use model::{Model, Node, Op};
use proptest::{
    prelude::*,
    test_runner::{Config as PropertyConfig, TestRunner},
};
use std::collections::BTreeMap;
struct Case {
    workspace: Uuid,
    actors: Vec<Credential>,
    sessions: Vec<Value>,
    wire_handles: BTreeMap<i64, Uuid>,
    inode_ids: BTreeMap<i64, i64>,
}
impl Case {
    async fn new(f: &Fixture) -> Self {
        let mut body = workspace_body();
        body["root_mode"] = json!(511);
        body["logical_inodes_limit"] = json!(1000);
        let created = f
            .client
            .call(
                "POST",
                "/v1/workspaces",
                Some(&f.admin),
                None,
                Some(&Uuid::new_v4().to_string()),
                Some(body),
            )
            .await
            .status(201)
            .body;
        let workspace =
            Uuid::parse_str(created["workspace"]["workspace_id"].as_str().unwrap()).unwrap();
        let mut actors = vec![];
        let mut sessions = vec![];
        for (kind, role, uid) in [
            ("worker", "write", 1000),
            ("worker", "write", 2000),
            ("host", "admin", 1000),
        ] {
            let grant=f.client.call("POST","/v1/credentials",Some(&f.admin),None,Some(&Uuid::new_v4().to_string()),Some(json!({"workspace_id":workspace,"kind":kind,"role":role,"uid":uid,"gid":1000,"max_reservation_bytes":4096}))).await.status(201).body;
            let credential = Credential::from_grant(&grant);
            let session = f
                .client
                .call(
                    "POST",
                    &format!("/v1/workspaces/{workspace}/sessions"),
                    Some(&f.admin),
                    None,
                    Some(&Uuid::new_v4().to_string()),
                    Some(json!({"credential_id":credential.id})),
                )
                .await
                .status(201)
                .body;
            actors.push(credential);
            sessions.push(session);
        }
        Self {
            workspace,
            actors,
            sessions,
            wire_handles: BTreeMap::new(),
            inode_ids: BTreeMap::from([(
                1,
                created["workspace"]["root"]["inode_id"].as_i64().unwrap(),
            )]),
        }
    }
    async fn apply(&mut self, f: &Fixture, model: &mut Model, op: &Op) {
        let selected = model.selected_handle(op.flags);
        let handle = selected
            .and_then(|id| self.wire_handles.get(&id).copied())
            .unwrap_or(Uuid::nil());
        let (method, suffix, body) = match op.kind {
            0 => (
                "POST",
                "/directories".into(),
                Some(json!({"path":op.path,"mode":op.mode})),
            ),
            1 => ("DELETE", format!("/directories?path={}", op.path), None),
            2 => (
                "POST",
                "/symlinks".into(),
                Some(json!({"path":op.path,"target":"/opaque"})),
            ),
            3 => (
                "POST",
                "/links".into(),
                Some(json!({"existing_path":op.path,"new_path":op.other})),
            ),
            4 => ("DELETE", format!("/files?path={}", op.path), None),
            5 => (
                "POST",
                "/rename".into(),
                Some(json!({"from":op.path,"to":op.other})),
            ),
            6 => (
                "PATCH",
                "/attributes".into(),
                Some(json!({"path":op.path,"mode":op.mode})),
            ),
            7 => (
                "PATCH",
                "/attributes".into(),
                Some(
                    json!({"path":op.path,"uid":if op.flags&1==0{1000}else{2000},"gid":if op.flags&2==0{1000}else{2000}}),
                ),
            ),
            8 => (
                "POST",
                "/handles".into(),
                Some(
                    json!({"path":op.path,"read":op.flags&1!=0,"write":op.flags&2!=0,"create":op.flags&4!=0,"exclusive":op.flags&8!=0,"truncate":op.flags&16!=0,"mode":op.mode}),
                ),
            ),
            9 => ("DELETE", format!("/handles/{handle}"), None),
            _ => unreachable!(),
        };
        let mut trial = model.clone();
        let expected = trial.apply(op);
        let reply = f
            .client
            .call(
                method,
                &format!("/v1/workspaces/{}{suffix}", self.workspace),
                Some(&self.actors[op.actor]),
                Some(&self.sessions[op.actor]),
                Some(&Uuid::new_v4().to_string()),
                body,
            )
            .await;
        match expected {
            Err(code) => {
                assert_eq!(reply.body["code"], code, "operation {op:?}: {}", reply.body);
                assert_eq!(
                    reply.status.as_u16(),
                    match code {
                        "forbidden" => 403,
                        "not_found" => 404,
                        "invalid_input" => 400,
                        _ => 409,
                    },
                    "{op:?}"
                );
            }
            Ok(node) => {
                assert_eq!(
                    reply.status.as_u16(),
                    if op.kind == 8 { 201 } else { 200 },
                    "{op:?}: {}",
                    reply.body
                );
                if let Some(node) = node {
                    let id = if op.kind == 8 {
                        reply.body["attributes"]["inode_id"].as_i64()
                    } else {
                        reply.body["inode_id"].as_i64()
                    }
                    .unwrap();
                    if let Some(previous) = self.inode_ids.insert(node, id) {
                        assert_eq!(previous, id, "inode identity changed: {op:?}");
                    }
                }
                if op.kind == 8 {
                    let new = *trial.handles.last_key_value().unwrap().0;
                    self.wire_handles.insert(
                        new,
                        Uuid::parse_str(reply.body["handle_id"].as_str().unwrap()).unwrap(),
                    );
                }
                if op.kind == 9 {
                    self.wire_handles.remove(&selected.unwrap());
                }
                *model = trial;
            }
        }
        self.verify(f, model).await;
    }
    fn attributes(&self, attrs: &Value, model: &Model, id: i64, node: &Node) {
        assert_eq!(attrs["inode_id"], self.inode_ids[&id]);
        assert_eq!(attrs["kind"], node.kind);
        assert_eq!(attrs["mode"], node.mode);
        assert_eq!(attrs["uid"], node.uid);
        assert_eq!(attrs["gid"], node.gid);
        assert_eq!(attrs["nlink"], model.nlink(id));
        assert_eq!(
            attrs["size"],
            node.target.as_ref().map_or(0, |s| s.len()) as u64
        );
    }
    async fn verify(&self, f: &Fixture, model: &Model) {
        for (path, id) in model.paths() {
            let query = if path.is_empty() {
                String::new()
            } else {
                format!("?path={path}")
            };
            let prefix = format!("/v1/workspaces/{}", self.workspace);
            let stat = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/stat{query}"),
                    Some(&self.actors[2]),
                    Some(&self.sessions[2]),
                    None,
                    None,
                )
                .await
                .status(200)
                .body;
            let node = &model.nodes[&id];
            self.attributes(&stat, model, id, node);
            if node.kind == "directory" {
                let page = f
                    .client
                    .call(
                        "GET",
                        &format!("{prefix}/directory{query}"),
                        Some(&self.actors[2]),
                        Some(&self.sessions[2]),
                        None,
                        None,
                    )
                    .await
                    .status(200)
                    .body;
                let entries = page["entries"].as_array().unwrap();
                assert_eq!(entries.len(), node.children.len());
                for (entry, (name, child)) in entries.iter().zip(&node.children) {
                    assert_eq!(entry["name"], *name);
                    self.attributes(&entry["attributes"], model, *child, &model.nodes[child]);
                }
            }
        }
        assert!(
            anchorfs_persistence::invariants::check(
                &f.pool,
                anchorfs_core::WorkspaceId(self.workspace)
            )
            .await
            .unwrap()
            .is_empty()
        );
        f.backend.untouched();
    }
}
fn path_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(0..4_usize, 1..=3).prop_map(|names| {
        names
            .into_iter()
            .map(|n| ["a", "b", "c", "d"][n])
            .collect::<Vec<_>>()
            .join("/")
    })
}
fn op_strategy() -> impl Strategy<Value = Op> {
    (
        0..3_usize,
        0..10_u8,
        path_strategy(),
        path_strategy(),
        prop::sample::select(vec![0, 0o600, 0o700, 0o755, 0o777, 0o4755]),
        0..32_u8,
    )
        .prop_map(|(actor, kind, path, other, mode, flags)| Op {
            actor,
            kind,
            path,
            other,
            mode,
            flags,
        })
}
#[test]
fn namespace_model_sequences() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _entered = runtime.enter();
    let fixture = runtime.block_on(Fixture::new());
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let mut runner = TestRunner::new(PropertyConfig {
        cases,
        max_shrink_iters: 256,
        failure_persistence: None,
        ..Default::default()
    });
    let started = std::time::Instant::now();
    runner
        .run(&prop::collection::vec(op_strategy(), 1..=16), |ops| {
            runtime.block_on(async {
                let mut case = Case::new(&fixture).await;
                let mut model = Model::new();
                // Seed varied kinds and permissions so generated mutations exercise existing paths.
                for op in [
                    Op {
                        actor: 2,
                        kind: 0,
                        path: "a".into(),
                        other: String::new(),
                        mode: 0o777,
                        flags: 0,
                    },
                    Op {
                        actor: 2,
                        kind: 8,
                        path: "b".into(),
                        other: String::new(),
                        mode: 0o666,
                        flags: 7,
                    },
                    Op {
                        actor: 2,
                        kind: 2,
                        path: "c".into(),
                        other: String::new(),
                        mode: 0,
                        flags: 0,
                    },
                    Op {
                        actor: 2,
                        kind: 0,
                        path: "d".into(),
                        other: String::new(),
                        mode: 0o700,
                        flags: 0,
                    },
                ] {
                    case.apply(&fixture, &mut model, &op).await;
                }
                for op in &ops {
                    case.apply(&fixture, &mut model, op).await;
                }
            });
            Ok(())
        })
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(300),
        "model exceeded five-minute budget"
    );
    eprintln!(
        "namespace model: {cases} generated sequences in {:?}",
        started.elapsed()
    );
}
