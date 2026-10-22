//! Test-private raw-JSON layer over `reqwest` for the routes `anchorfs-client` does not wrap:
//! workspace and credential administration, session minting with lease binding, workflow
//! leases, and the session negatives. Shapes follow `docs/api/openapi-v1.yaml`; the in-process
//! helpers under `tests/common` are shape references only (they use axum `oneshot`).
//! Debug output of secret-bearing types is redacted.
use serde_json::{Value, json};
use std::{fmt, sync::Arc, time::Duration};
use tokio::runtime::Runtime;
use uuid::Uuid;

/// `<credential_id>.<secret>`; the secret part is 64 lowercase hex characters.
#[derive(Clone)]
pub struct Bearer {
    pub credential_id: Uuid,
    value: String,
}
impl Bearer {
    pub fn compose(credential_id: Uuid, secret: &str) -> Self {
        assert!(
            secret.len() == 64
                && secret
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "credential secret is not 64 lowercase hex characters"
        );
        Self {
            credential_id,
            value: format!("{credential_id}.{secret}"),
        }
    }
    /// Parses the owner-only bootstrap JSON `{"credential_id","secret","credential_epoch"}`.
    pub fn from_bootstrap_json(bytes: &[u8]) -> Self {
        let value: Value = serde_json::from_slice(bytes).expect("bootstrap credential JSON");
        let id = value["credential_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .expect("bootstrap credential_id");
        Self::compose(id, value["secret"].as_str().expect("bootstrap secret"))
    }
    /// The composed value, for Secret creation and request headers only.
    pub fn expose(&self) -> &str {
        &self.value
    }
}
impl fmt::Debug for Bearer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bearer({}.[redacted])", self.credential_id)
    }
}

#[derive(Clone, Debug)]
pub struct SessionRef {
    pub session_id: Uuid,
    pub session_epoch: i64,
}
#[derive(Clone)]
pub struct Lease {
    pub token: String,
    pub epoch: i64,
}
impl fmt::Debug for Lease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lease(epoch {}, token [redacted])", self.epoch)
    }
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}
impl Reply {
    pub fn expect(self, status: u16, what: &str) -> Self {
        assert_eq!(
            self.status, status,
            "{what}: expected HTTP {status}, got {} code={} message={}",
            self.status, self.body["code"], self.body["message"]
        );
        self
    }
}

/// Where a lost mutation response is resolved from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Admin,
    Workspace(Uuid),
}

pub struct Api {
    runtime: Arc<Runtime>,
    http: reqwest::Client,
    base: String,
}
impl Api {
    pub fn new(runtime: Arc<Runtime>, base: &str, ca_pem: &[u8]) -> Self {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30));
        for cert in reqwest::Certificate::from_pem_bundle(ca_pem).expect("CA bundle") {
            builder = builder.add_root_certificate(cert);
        }
        Self {
            runtime,
            http: builder.build().expect("http client"),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_once(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        bearer: &Bearer,
        session: Option<&SessionRef>,
        lease: Option<&Lease>,
        key: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Reply, String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .query(query)
            .bearer_auth(bearer.expose());
        if let Some(session) = session {
            request = request
                .header("X-Anchorfs-Session-Id", session.session_id.to_string())
                .header("X-Anchorfs-Session-Epoch", session.session_epoch);
        }
        if let Some(lease) = lease {
            request = request
                .header("X-Anchorfs-Workflow-Token", lease.token.as_str())
                .header("X-Anchorfs-Workflow-Epoch", lease.epoch);
        }
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        self.runtime.block_on(async {
            let response = request
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let status = response.status().as_u16();
            let bytes = response
                .bytes()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            Ok(Reply { status, body })
        })
    }

    /// A mutation with a stable idempotency key. A transport failure or a 503 is resolved
    /// through the operation lookup; when nothing was recorded the same key is replayed once.
    #[allow(clippy::too_many_arguments)]
    fn mutate(
        &self,
        method: reqwest::Method,
        path: &str,
        scope: Scope,
        bearer: &Bearer,
        session: Option<&SessionRef>,
        lease: Option<&Lease>,
        body: Option<&Value>,
    ) -> Reply {
        let key = Uuid::new_v4().to_string();
        let first = self.send_once(
            method.clone(),
            path,
            &[],
            bearer,
            session,
            lease,
            Some(&key),
            body,
        );
        match first {
            Ok(reply) if reply.status != 503 => return reply,
            _ => {}
        }
        let lookup_path = match scope {
            Scope::Admin => "/v1/admin-operations".to_owned(),
            Scope::Workspace(id) => format!("/v1/workspaces/{id}/operations"),
        };
        for _ in 0..3 {
            if let Ok(found) = self.send_once(
                reqwest::Method::GET,
                &lookup_path,
                &[("idempotency_key", key.as_str())],
                bearer,
                session,
                None,
                None,
                None,
            ) {
                if found.status == 200 && found.body["state"] == "committed" {
                    // The original body is not replayed for secrets; callers check for them.
                    return Reply {
                        status: 200,
                        body: found.body["result"].clone(),
                    };
                }
                if found.status == 200 && found.body["state"] == "failed" {
                    return Reply {
                        status: 409,
                        body: found.body,
                    };
                }
            }
            let replay = self.send_once(
                method.clone(),
                path,
                &[],
                bearer,
                session,
                lease,
                Some(&key),
                body,
            );
            if let Some(reply) = replay.ok().filter(|reply| reply.status != 503) {
                return reply;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        panic!("mutation {method} {path}: outcome unresolved after lookup and replay");
    }

    pub fn create_workspace(&self, admin: &Bearer, root_uid: u32, root_gid: u32) -> Uuid {
        let body = json!({
            "root_uid": root_uid,
            "root_gid": root_gid,
            "logical_bytes_limit": 256_u64 << 20,
            "logical_inodes_limit": 10_000,
            "physical_bytes_limit": 512_u64 << 20,
            "staging_bytes_limit": 64_u64 << 20,
        });
        let reply = self
            .mutate(
                reqwest::Method::POST,
                "/v1/workspaces",
                Scope::Admin,
                admin,
                None,
                None,
                Some(&body),
            )
            .expect(201, "createWorkspace");
        reply.body["workspace"]["workspace_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .expect("workspace_id in createWorkspace response")
    }

    /// `kind` is `host` or `worker`; the role is `write`.
    pub fn create_credential(
        &self,
        actor: &Bearer,
        workspace: Uuid,
        kind: &str,
        uid: u32,
        gid: u32,
    ) -> Bearer {
        let body = json!({
            "workspace_id": workspace,
            "kind": kind,
            "role": "write",
            "uid": uid,
            "gid": gid,
            "max_reservation_bytes": 4_194_304_u64,
        });
        let reply = self
            .mutate(
                reqwest::Method::POST,
                "/v1/credentials",
                Scope::Admin,
                actor,
                None,
                None,
                Some(&body),
            )
            .expect(201, "createCredential");
        let id = reply.body["credential_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .expect("credential_id");
        let secret = reply.body["secret"]
            .as_str()
            .expect("credential secret missing (a lost response cannot be replayed)");
        Bearer::compose(id, secret)
    }

    pub fn revoke_credential(&self, admin: &Bearer, credential: Uuid) {
        self.mutate(
            reqwest::Method::DELETE,
            &format!("/v1/credentials/{credential}"),
            Scope::Admin,
            admin,
            None,
            None,
            None,
        )
        .expect(200, "revokeCredential");
    }

    /// Mints a session for `credential_id`, acting as `actor` (a host credential, or the
    /// credential itself). `bind` is the workflow lease token for a bound successor.
    pub fn create_session(
        &self,
        actor: &Bearer,
        workspace: Uuid,
        credential_id: Uuid,
        bind: Option<&str>,
    ) -> SessionRef {
        let body = json!({"credential_id": credential_id, "bind_to_workflow_lease": bind});
        let reply = self
            .mutate(
                reqwest::Method::POST,
                &format!("/v1/workspaces/{workspace}/sessions"),
                Scope::Admin,
                actor,
                None,
                None,
                Some(&body),
            )
            .expect(201, "createSession");
        session_from(&reply.body)
    }

    /// Raw createSession for negatives: returns the reply instead of asserting.
    pub fn create_session_raw(
        &self,
        actor: &Bearer,
        workspace: Uuid,
        credential_id: Uuid,
    ) -> Reply {
        let body = json!({"credential_id": credential_id, "bind_to_workflow_lease": null});
        self.send_once(
            reqwest::Method::POST,
            &format!("/v1/workspaces/{workspace}/sessions"),
            &[],
            actor,
            None,
            None,
            Some(&Uuid::new_v4().to_string()),
            Some(&body),
        )
        .expect("createSession transport")
    }

    pub fn acquire_lease(&self, host: &Bearer, session: &SessionRef, workspace: Uuid) -> Lease {
        let reply = self
            .mutate(
                reqwest::Method::POST,
                &format!("/v1/workspaces/{workspace}/workflow-lease"),
                Scope::Workspace(workspace),
                host,
                Some(session),
                None,
                None,
            )
            .expect(201, "acquireWorkflowLease");
        Lease {
            token: reply.body["token"]
                .as_str()
                .expect("lease token missing (a lost response cannot be replayed)")
                .to_owned(),
            epoch: reply.body["mutation_epoch"]
                .as_i64()
                .expect("mutation_epoch"),
        }
    }

    pub fn acquire_lease_raw(
        &self,
        actor: &Bearer,
        session: &SessionRef,
        workspace: Uuid,
    ) -> Reply {
        self.send_once(
            reqwest::Method::POST,
            &format!("/v1/workspaces/{workspace}/workflow-lease"),
            &[],
            actor,
            Some(session),
            None,
            Some(&Uuid::new_v4().to_string()),
            None,
        )
        .expect("acquireWorkflowLease transport")
    }

    pub fn release_lease(
        &self,
        host: &Bearer,
        session: &SessionRef,
        workspace: Uuid,
        lease: &Lease,
    ) {
        self.mutate(
            reqwest::Method::DELETE,
            &format!("/v1/workspaces/{workspace}/workflow-lease"),
            Scope::Workspace(workspace),
            host,
            Some(session),
            Some(lease),
            None,
        )
        .expect(200, "releaseWorkflowLease");
    }

    pub fn renew_session_raw(
        &self,
        actor: &Bearer,
        session: &SessionRef,
        workspace: Uuid,
    ) -> Reply {
        self.send_once(
            reqwest::Method::POST,
            &format!(
                "/v1/workspaces/{workspace}/sessions/{}/renew",
                session.session_id
            ),
            &[],
            actor,
            Some(session),
            None,
            Some(&Uuid::new_v4().to_string()),
            None,
        )
        .expect("renewSession transport")
    }

    pub fn rebind_session_raw(
        &self,
        actor: &Bearer,
        session: &SessionRef,
        workspace: Uuid,
    ) -> Reply {
        self.send_once(
            reqwest::Method::POST,
            &format!(
                "/v1/workspaces/{workspace}/sessions/{}/rebind",
                session.session_id
            ),
            &[],
            actor,
            Some(session),
            None,
            Some(&Uuid::new_v4().to_string()),
            Some(&json!({"workflow_lease_token": null})),
        )
        .expect("rebindSession transport")
    }
}

fn session_from(body: &Value) -> SessionRef {
    SessionRef {
        session_id: body["session_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .expect("session_id"),
        session_epoch: body["session_epoch"].as_i64().expect("session_epoch"),
    }
}
