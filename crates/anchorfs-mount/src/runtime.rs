//! Linux mount authority, identity checks, and renewal. No server clocks drive timers.
use crate::{
    MountError,
    config::MountConfig,
    errno::{CallClass, ErrnoName, errno_for},
    handles::*,
    inodes::InodeTable,
};
use anchorfs_client::{
    Client, auth::SessionAuthority, error::ClientError, retry::IdempotencyKey, wire::*,
};
use anchorfs_core::{Name, SessionId, WorkspacePath};
use std::{
    future::Future,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
pub struct Mount {
    pub client: Client,
    pub cfg: MountConfig,
    pub inodes: Mutex<InodeTable>,
    pub files: Mutex<OpenFiles>,
    pub dirs: Mutex<OpenDirs>,
    pub fence: AtomicBool,
    pub stopping: AtomicBool,
    pub rt: tokio::runtime::Handle,
    self_minted: bool,
}
impl Mount {
    pub async fn start(
        cfg: MountConfig,
        rt: tokio::runtime::Handle,
    ) -> Result<Arc<Self>, MountError> {
        let auth = |e: ClientError| MountError::Authorization(e.to_string());
        let mut client = Client::new(
            cfg.endpoint.clone(),
            cfg.args.workspace,
            cfg.authority.clone(),
            cfg.ca_bundle.as_deref(),
        )
        .map_err(auth)?;
        client.metadata_deadline = Duration::from_millis(cfg.args.metadata_deadline_ms);
        client.data_deadline = Duration::from_millis(cfg.args.data_deadline_ms);
        let self_minted = client.authority().session.is_none();
        if self_minted {
            let grant = client
                .create_session(
                    client.authority().bearer.credential_id.0,
                    &IdempotencyKey::new(),
                )
                .await
                .map_err(|e| {
                    if e.code() == Some("forbidden") {
                        MountError::Authorization(
                            "worker credentials require a host-minted session".into(),
                        )
                    } else {
                        auth(e)
                    }
                })?;
            client
                .authority
                .write()
                .unwrap_or_else(|p| p.into_inner())
                .session = Some(SessionAuthority {
                session_id: SessionId(grant.session_id),
                session_epoch: grant.session_epoch,
            });
        }
        let root = client.get_workspace().await.map_err(auth)?;
        let probe = client.stat(None).await.map_err(auth)?;
        if probe.inode_id != root.root.inode_id {
            return Err(MountError::Authorization(
                "workspace root changed during startup".into(),
            ));
        }
        if let Some(w) = client.authority().workflow {
            if !root
                .workflow
                .is_some_and(|live| live.active && live.mutation_epoch == w.epoch)
            {
                return Err(MountError::Authorization(
                    "workflow epoch does not match a live lease".into(),
                ));
            }
        }
        let session = client.authority().session.expect("session established");
        client
            .renew_session(session.session_id.0, &IdempotencyKey::new())
            .await
            .map_err(auth)?;
        if !cfg.args.allow_other && (nix_uid() != probe.uid || nix_gid() != probe.gid) {
            tracing::warn!(
                "mount uid/gid differ from workspace root; default_permissions can reject the caller"
            );
        }
        let mount = Arc::new(Self {
            client,
            cfg,
            inodes: Mutex::new(InodeTable::new(root.root.inode_id)),
            files: Mutex::new(OpenFiles::default()),
            dirs: Mutex::new(OpenDirs::default()),
            fence: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            rt,
            self_minted,
        });
        let weak = Arc::downgrade(&mount);
        mount.rt.spawn(async move {
            let mut session_at = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(mount) = weak.upgrade() else { break };
                if mount.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if mount.fence.load(Ordering::SeqCst) {
                    continue;
                }
                if session_at.elapsed()
                    >= Duration::from_millis(mount.cfg.args.session_ttl_hint_ms / 2)
                {
                    let session = mount.client.authority().session.expect("session");
                    let result = mount
                        .client
                        .renew_session(session.session_id.0, &IdempotencyKey::new())
                        .await;
                    if let Err(error) = result {
                        mount.api_error(error, CallClass::Read, None).await;
                    }
                    session_at = Instant::now();
                }
                let renew: Vec<_> = lock(&mount.files)
                    .entries
                    .iter()
                    .filter(|(_, f)| {
                        !f.fenced
                            && f.renewed.elapsed()
                                >= Duration::from_millis(mount.cfg.args.handle_ttl_hint_ms / 2)
                    })
                    .map(|(fh, f)| (*fh, f.handle_id))
                    .collect();
                for (fh, id) in renew {
                    let result = mount.client.renew_handle(id, &IdempotencyKey::new()).await;
                    match result {
                        Ok(_) => {
                            if let Some(f) = lock(&mount.files).entries.get_mut(&fh) {
                                f.renewed = Instant::now();
                            }
                        }
                        Err(error) => {
                            mount.api_error(error, CallClass::Read, Some(fh)).await;
                        }
                    }
                }
            }
        });
        Ok(mount)
    }
    pub fn block<T>(&self, future: impl Future<Output = T>) -> T {
        self.rt.block_on(future)
    }
    pub fn live(&self) -> Result<(), ErrnoName> {
        if self.fence.load(Ordering::SeqCst) {
            Err(ErrnoName::ESTALE)
        } else {
            Ok(())
        }
    }
    pub async fn api<T>(
        &self,
        result: Result<T, ClientError>,
        class: CallClass,
        fh: Option<u64>,
    ) -> Result<T, ErrnoName> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => Err(self.api_error(error, class, fh).await),
        }
    }
    pub async fn api_error(
        &self,
        error: ClientError,
        class: CallClass,
        fh: Option<u64>,
    ) -> ErrnoName {
        if let ClientError::Api(e) = &error {
            tracing::warn!(status=e.status,code=%e.code,trace_id=%e.trace_id,operation_id=?e.operation_id,"native API call failed");
            if e.status == 401 {
                self.fence.store(true, Ordering::SeqCst);
            }
            if e.code == "stale_handle"
                || (e.code == "fenced" && fh.is_some() && matches!(class, CallClass::Read))
            {
                if let Some(id) = fh {
                    if let Some(file) = lock(&self.files).entries.get_mut(&id) {
                        file.fenced = true;
                    }
                }
            }
            if e.code == "fenced" {
                let bound_mutation = self.client.authority().workflow.is_some()
                    && matches!(class, CallClass::Mutation)
                    && e.message != "reservation_expired";
                let probe = self.client.stat(None).await;
                let dead =
                    matches!(&probe,Err(ClientError::Api(p)) if p.status==401 || p.code=="fenced");
                if bound_mutation || dead {
                    self.fence.store(true, Ordering::SeqCst);
                }
            }
        }
        errno_for(&error, class)
    }
    pub fn file(&self, fh: u64) -> Result<OpenFile, ErrnoName> {
        self.live()?;
        lock(&self.files)
            .entries
            .get(&fh)
            .filter(|f| !f.fenced)
            .cloned()
            .ok_or(ErrnoName::ESTALE)
    }
    pub async fn checked_path(&self, ino: u64) -> Result<Option<WorkspacePath>, ErrnoName> {
        self.live()?;
        let path = lock(&self.inodes).path(ino)?;
        let result = self
            .client
            .stat(path.as_ref().map(WorkspacePath::as_str))
            .await;
        let attributes = match self.api(result, CallClass::Read, None).await {
            Err(ErrnoName::ENOENT) => {
                lock(&self.inodes).detach(ino);
                return Err(ErrnoName::ESTALE);
            }
            other => other?,
        };
        self.identity(ino, attributes.inode_id)?;
        Ok(path)
    }
    pub fn identity(&self, ino: u64, server: i64) -> Result<(), ErrnoName> {
        let mut table = lock(&self.inodes);
        if table.server_id(ino) != server {
            table.detach(ino);
            Err(ErrnoName::ESTALE)
        } else {
            Ok(())
        }
    }
    pub async fn child_path(&self, parent: u64, name: &Name) -> Result<String, ErrnoName> {
        let parent = self.checked_path(parent).await?;
        let path = parent.map_or_else(
            || name.as_str().to_owned(),
            |p| format!("{p}/{}", name.as_str()),
        );
        WorkspacePath::parse(&path).map_err(|_| ErrnoName::ENAMETOOLONG)?;
        Ok(path)
    }
    pub async fn attributes(
        &self,
        ino: u64,
        fh: Option<u64>,
    ) -> Result<InodeAttributes, ErrnoName> {
        self.live()?;
        let attributes = if let Some(fh) = fh {
            let file = self.file(fh)?;
            self.api(
                self.client.fsync(file.handle_id).await,
                CallClass::Read,
                Some(fh),
            )
            .await?
        } else {
            let path = self.checked_path(ino).await?;
            self.api(
                self.client
                    .stat(path.as_ref().map(WorkspacePath::as_str))
                    .await,
                CallClass::Read,
                None,
            )
            .await?
        };
        self.identity(ino, attributes.inode_id)?;
        Ok(attributes)
    }
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let files = std::mem::take(&mut lock(&self.files).entries);
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            for (_, f) in files {
                let _ = self
                    .client
                    .close_handle(f.handle_id, &IdempotencyKey::new())
                    .await;
            }
            if self.self_minted {
                let session = self.client.authority().session.expect("session");
                let _ = self
                    .client
                    .revoke_session(session.session_id.0, &IdempotencyKey::new())
                    .await;
            }
        })
        .await;
    }
}
// Reading effective ids from proc avoids an unsafe block in production code.
fn effective_id(field: &str) -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(2))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(u32::MAX)
}
fn nix_uid() -> u32 {
    effective_id("Uid:")
}
fn nix_gid() -> u32 {
    effective_id("Gid:")
}
