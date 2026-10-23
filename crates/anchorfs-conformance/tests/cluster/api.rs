//! Test-private raw-JSON layer over `reqwest` for the routes `anchorfs-client` does not wrap:
//! workspace and credential administration, bound session minting, workflow leases and
//! rebind. Unbound `createSession` and `renewSession` go through the typed `Client`. Requests
//! carry `anchorfs_client::auth::Authority`, so headers and secret redaction have one owner.
//! Shapes follow `docs/api/openapi-v1.yaml`.
use crate::kube::PortForward;
use anchorfs_client::{
    Client,
    auth::{Authority, Bearer, SessionAuthority, WorkflowAuthority},
    endpoint::Endpoint,
    error::ClientError,
    retry::IdempotencyKey,
    wire::SessionGrant,
};
use anchorfs_core::{Secret, SessionId};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::runtime::Runtime;
use uuid::Uuid;

/// A composed `<credential_id>.<secret>` bearer. The composed text is kept for Kubernetes
/// Secret creation only; both parts redact in `Debug`.
#[derive(Clone, Debug)]
pub struct Credential {
    bearer: Bearer,
    composed: Secret<String>,
}
impl Credential {
    pub fn compose(credential_id: Uuid, secret: &str) -> Self {
        let composed = format!("{credential_id}.{secret}");
        let bearer = Bearer::parse(&composed).expect("credential secret is not a valid bearer");
        Self {
            bearer,
            composed: Secret::new(composed),
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
    pub fn id(&self) -> Uuid {
        self.bearer.credential_id.0
    }
    /// The composed value, for Secret creation only.
    pub fn composed(&self) -> &str {
        self.composed.expose()
    }
    pub fn authority(&self, session: Option<&SessionAuthority>) -> Authority {
        Authority {
            bearer: self.bearer.clone(),
            session: session.cloned(),
            workflow: None,
        }
    }
}

pub fn session_of(grant: &SessionGrant) -> SessionAuthority {
    SessionAuthority {
        session_id: SessionId(grant.session_id),
        session_epoch: grant.session_epoch,
    }
}

/// HTTP status of a typed-client outcome, for the negative checks (success reads as 200).
pub fn status_of<T>(result: Result<T, ClientError>) -> u16 {
    match result {
        Ok(_) => 200,
        Err(ClientError::Api(failure)) => failure.status,
        Err(error) => panic!("negative check failed without an HTTP status: {error}"),
    }
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
    /// True when the original response was lost and this reply was rebuilt from the committed
    /// operation receipt (HTTP 200 from the lookup, whatever the original success status was).
    pub recovered: bool,
}
impl Reply {
    pub fn expect(self, status: u16, what: &str) -> Self {
        // A recovered receipt stands for the original success; only the lookup's 200 is seen.
        let observed = if self.recovered && self.status == 200 && (200..300).contains(&status) {
            status
        } else {
            self.status
        };
        assert_eq!(
            observed, status,
            "{what}: expected HTTP {status}, got {} code={} message={}",
            self.status, self.body["code"], self.body["message"]
        );
        self
    }
}

/// Where a lost mutation response is resolved from; mirrors the service's receipt scope and
/// the client's `Resolver` for the same route.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Admin,
    Workspace(Uuid),
}

/// One request: what to send and as whom.
pub struct Call<'a> {
    method: Method,
    path: String,
    authority: &'a Authority,
    body: Option<Value>,
}
impl<'a> Call<'a> {
    pub fn new(method: Method, path: impl Into<String>, authority: &'a Authority) -> Self {
        Self {
            method,
            path: path.into(),
            authority,
            body: None,
        }
    }
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

pub struct Api {
    runtime: Arc<Runtime>,
    forward: Arc<Mutex<PortForward>>,
    ca: Vec<u8>,
    http: reqwest::Client,
}
impl Api {
    pub fn new(runtime: Arc<Runtime>, forward: Arc<Mutex<PortForward>>, ca: Vec<u8>) -> Self {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30));
        for cert in reqwest::Certificate::from_pem_bundle(&ca).expect("CA bundle") {
            builder = builder.add_root_certificate(cert);
        }
        Self {
            runtime,
            forward,
            ca,
            http: builder.build().expect("http client"),
        }
    }

    pub fn ca(&self) -> &[u8] {
        &self.ca
    }

    pub fn forward_stderr(&self) -> String {
        self.forward
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stderr_text()
    }

    fn origin(&self) -> String {
        let port = self
            .forward
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .local_port;
        // The forward binds IPv4 loopback only and the service certificate carries the IP SAN
        // `127.0.0.1`, so the literal address is used (no `::1` attempt).
        format!("https://127.0.0.1:{port}")
    }

    pub fn endpoint(&self) -> Endpoint {
        Endpoint::parse(&self.origin(), false).expect("endpoint")
    }

    pub fn client(&self, workspace: Uuid, authority: Authority) -> Client {
        Client::new(self.endpoint(), workspace, authority, Some(&self.ca)).expect("client")
    }

    /// Restarts the port-forward if its child exited.
    fn ensure_forward(&self) {
        self.forward
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ensure_alive();
    }

    /// Every typed-client call goes through here: a dropped forward is restarted first.
    pub fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.ensure_forward();
        self.runtime.block_on(future)
    }

    fn send(&self, call: &Call<'_>, key: Option<&str>) -> Result<Reply, String> {
        self.ensure_forward();
        let url = format!("{}{}", self.origin(), call.path);
        let mut request = call
            .authority
            .apply(self.http.request(call.method.clone(), url));
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(body) = &call.body {
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
            Ok(Reply {
                status,
                body,
                recovered: false,
            })
        })
    }

    /// One request with a fresh idempotency key and no recovery, for the negative checks.
    pub fn raw(&self, call: Call<'_>) -> Reply {
        self.send(&call, Some(&Uuid::new_v4().to_string()))
            .unwrap_or_else(|e| panic!("{} {} transport: {e}", call.method, call.path))
    }

    /// A mutation with a stable idempotency key. A transport failure or a 503 is resolved
    /// through the operation lookup; when nothing was recorded the same key is replayed once.
    pub fn mutate(&self, call: Call<'_>, scope: Scope) -> Reply {
        let key = Uuid::new_v4().to_string();
        match self.send(&call, Some(&key)) {
            Ok(reply) if reply.status != 503 => return reply,
            _ => {}
        }
        let lookup_authority = Authority {
            workflow: None,
            ..call.authority.clone()
        };
        let lookup_path = match scope {
            Scope::Admin => format!("/v1/admin-operations?idempotency_key={key}"),
            Scope::Workspace(id) => format!("/v1/workspaces/{id}/operations?idempotency_key={key}"),
        };
        for _ in 0..3 {
            let lookup = Call::new(Method::GET, lookup_path.as_str(), &lookup_authority);
            if let Ok(found) = self.send(&lookup, None) {
                if found.status == 200 && found.body["state"] == "committed" {
                    // The original body is not replayed for secrets; callers check for them.
                    return Reply {
                        status: 200,
                        body: found.body["result"].clone(),
                        recovered: true,
                    };
                }
                if found.status == 200 && found.body["state"] == "failed" {
                    return Reply {
                        status: 409,
                        body: found.body,
                        recovered: true,
                    };
                }
            }
            if let Some(reply) = self
                .send(&call, Some(&key))
                .ok()
                .filter(|reply| reply.status != 503)
            {
                return reply;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        panic!(
            "mutation {} {}: outcome unresolved after lookup and replay",
            call.method, call.path
        );
    }

    pub fn create_workspace(&self, admin: &Authority, root_uid: u32, root_gid: u32) -> Uuid {
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
                Call::new(Method::POST, "/v1/workspaces", admin).json(body),
                Scope::Admin,
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
        actor: &Authority,
        workspace: Uuid,
        kind: &str,
        uid: u32,
        gid: u32,
    ) -> Credential {
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
                Call::new(Method::POST, "/v1/credentials", actor).json(body),
                Scope::Admin,
            )
            .expect(201, "createCredential");
        let id = reply.body["credential_id"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .expect("credential_id");
        let secret = reply.body["secret"]
            .as_str()
            .expect("credential secret missing (a lost response cannot be replayed)");
        Credential::compose(id, secret)
    }

    pub fn revoke_credential(&self, admin: &Authority, credential: Uuid) {
        self.mutate(
            Call::new(
                Method::DELETE,
                format!("/v1/credentials/{credential}"),
                admin,
            ),
            Scope::Admin,
        )
        .expect(200, "revokeCredential");
    }

    /// Mints an unbound session for `credential_id`, acting as `actor`, through the typed
    /// client (which resolves a lost response from the admin-operation lookup).
    pub fn create_session(
        &self,
        actor: &Authority,
        workspace: Uuid,
        credential_id: Uuid,
    ) -> SessionAuthority {
        let client = self.client(workspace, actor.clone());
        let grant = self
            .block_on(client.create_session(credential_id, &IdempotencyKey::new()))
            .unwrap_or_else(|e| panic!("createSession: {e}"));
        session_of(&grant)
    }

    /// Mints a session bound to a workflow lease token (not wrapped by the typed client).
    pub fn create_bound_session(
        &self,
        actor: &Authority,
        workspace: Uuid,
        credential_id: Uuid,
        lease_token: &str,
    ) -> SessionAuthority {
        let body = json!({"credential_id": credential_id, "bind_to_workflow_lease": lease_token});
        let reply = self
            .mutate(
                Call::new(
                    Method::POST,
                    format!("/v1/workspaces/{workspace}/sessions"),
                    actor,
                )
                .json(body),
                Scope::Admin,
            )
            .expect(201, "createSession");
        SessionAuthority {
            session_id: SessionId(
                reply.body["session_id"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .expect("session_id"),
            ),
            session_epoch: reply.body["session_epoch"].as_i64().expect("session_epoch"),
        }
    }

    /// `host` carries the owner's unbound session.
    pub fn acquire_lease(&self, host: &Authority, workspace: Uuid) -> WorkflowAuthority {
        let reply = self
            .mutate(
                Call::new(
                    Method::POST,
                    format!("/v1/workspaces/{workspace}/workflow-lease"),
                    host,
                ),
                Scope::Workspace(workspace),
            )
            .expect(201, "acquireWorkflowLease");
        WorkflowAuthority {
            token: Secret::new(
                reply.body["token"]
                    .as_str()
                    .expect("lease token missing (a lost response cannot be replayed)")
                    .to_owned(),
            ),
            epoch: reply.body["mutation_epoch"]
                .as_i64()
                .expect("mutation_epoch"),
        }
    }

    pub fn acquire_lease_raw(&self, actor: &Authority, workspace: Uuid) -> Reply {
        self.raw(Call::new(
            Method::POST,
            format!("/v1/workspaces/{workspace}/workflow-lease"),
            actor,
        ))
    }

    pub fn release_lease(&self, host: &Authority, workspace: Uuid, lease: &WorkflowAuthority) {
        let leased = leased(host, lease);
        self.mutate(
            Call::new(
                Method::DELETE,
                format!("/v1/workspaces/{workspace}/workflow-lease"),
                &leased,
            ),
            Scope::Workspace(workspace),
        )
        .expect(200, "releaseWorkflowLease");
    }

    /// Renews the workflow lease TTL (owner host credential, unbound session, lease proof).
    /// The mutation epoch is unchanged by a renewal; it is returned for the caller to check.
    pub fn renew_lease(&self, host: &Authority, workspace: Uuid, lease: &WorkflowAuthority) -> i64 {
        let leased = leased(host, lease);
        let reply = self
            .mutate(
                Call::new(
                    Method::POST,
                    format!("/v1/workspaces/{workspace}/workflow-lease/renew"),
                    &leased,
                ),
                Scope::Workspace(workspace),
            )
            .expect(200, "renewWorkflowLease");
        let recovered = reply.recovered;
        reply.body["mutation_epoch"]
            .as_i64()
            .or(recovered.then_some(lease.epoch))
            .expect("mutation_epoch in renewWorkflowLease response")
    }

    pub fn rebind_session_raw(
        &self,
        actor: &Authority,
        workspace: Uuid,
        session: &SessionAuthority,
    ) -> Reply {
        self.raw(
            Call::new(
                Method::POST,
                format!(
                    "/v1/workspaces/{workspace}/sessions/{}/rebind",
                    session.session_id.0
                ),
                actor,
            )
            .json(json!({"workflow_lease_token": null})),
        )
    }
}

/// `authority` plus the workflow lease proof.
fn leased(authority: &Authority, lease: &WorkflowAuthority) -> Authority {
    Authority {
        workflow: Some(lease.clone()),
        ..authority.clone()
    }
}
