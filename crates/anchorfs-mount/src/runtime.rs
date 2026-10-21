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
        client = client.with_deadlines(
            Duration::from_millis(cfg.args.metadata_deadline_ms),
            Duration::from_millis(cfg.args.data_deadline_ms),
        );
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
            let mut authority = client.authority();
            authority.session = Some(SessionAuthority {
                session_id: SessionId(grant.session_id),
                session_epoch: grant.session_epoch,
            });
            client = client.with_authority(authority);
        }
        let startup = async {
        let root = client.get_workspace().await.map_err(auth)?;
        let probe = client.stat(None).await.map_err(auth)?;
        if probe.inode_id != root.root.inode_id {
            return Err(MountError::Authorization(
                "workspace root changed during startup".into(),
            ));
        }
        if let Some(w) = client.authority().workflow
            && !root
                .workflow
                .as_ref()
                .is_some_and(|live| live.active && live.mutation_epoch == w.epoch)
        {
            return Err(MountError::Authorization(
                "workflow epoch does not match a live lease".into(),
            ));
        }
        let session = client.authority().session.expect("session established");
        let renewed = client
            .renew_session(session.session_id.0, &IdempotencyKey::new())
            .await
            .map_err(auth)?;
        tracing::info!(expires_at=%renewed.expires_at, "mount session renewed at startup");
        if !cfg.args.allow_other && (nix_uid() != probe.uid || nix_gid() != probe.gid) {
            tracing::warn!(
                "mount uid/gid differ from workspace root; default_permissions can reject the caller"
            );
        }
        Ok::<_, MountError>((root, probe))
        }.await;
        let (root, _probe) = match startup {
            Ok(value) => value,
            Err(error) => {
                if self_minted {
                    let session = client.authority().session.expect("session established");
                    let _ = tokio::time::timeout(
                        Duration::from_secs(10),
                        client.revoke_session(session.session_id.0, &IdempotencyKey::new()),
                    )
                    .await;
                }
                return Err(error);
            }
        };
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
        // Session renewal has its own task: slow handle renewals cannot starve it.
        let weak = Arc::downgrade(&mount);
        mount.rt.spawn(async move {
            let mut session_at = Instant::now();
            let mut next_try = Instant::now();
            let mut backoff = Duration::from_millis(250);
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(mount) = weak.upgrade() else { break };
                if mount.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if mount.fence.load(Ordering::SeqCst) {
                    continue;
                }
                let ttl = Duration::from_millis(mount.cfg.args.session_ttl_hint_ms);
                if session_at.elapsed() < ttl / 2 || Instant::now() < next_try {
                    continue;
                }
                let session = mount.client.authority().session.expect("session");
                match mount
                    .client
                    .renew_session(session.session_id.0, &IdempotencyKey::new())
                    .await
                {
                    Ok(_) => {
                        session_at = Instant::now();
                        backoff = Duration::from_millis(250);
                    }
                    Err(error) => {
                        let delay = renewal_delay(&error, backoff);
                        next_try = Instant::now() + delay;
                        backoff = (backoff * 2).min(
                            (ttl / 8).clamp(Duration::from_millis(250), Duration::from_secs(5)),
                        );
                        mount.api_error(error, CallClass::Read, None).await;
                    }
                }
            }
        });
        let weak = Arc::downgrade(&mount);
        mount.rt.spawn(async move {
            let mut retries = std::collections::BTreeMap::<u64, (Instant, Duration)>::new();
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(mount) = weak.upgrade() else { break };
                if mount.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if mount.fence.load(Ordering::SeqCst) {
                    continue;
                }
                let due = Duration::from_millis(mount.cfg.args.handle_ttl_hint_ms / 2);
                let renew: Vec<_> = {
                    let files = lock(&mount.files);
                    retries.retain(|fh, _| files.entries.contains_key(fh));
                    files
                        .entries
                        .iter()
                        .filter(|(fh, f)| {
                            !f.fenced
                                && f.renewed.elapsed() >= due
                                && retries.get(fh).is_none_or(|(at, _)| Instant::now() >= *at)
                        })
                        .take(8)
                        .map(|(fh, f)| (*fh, f.handle_id))
                        .collect()
                };
                let mut jobs = tokio::task::JoinSet::new();
                for (fh, id) in renew {
                    let mount = mount.clone();
                    jobs.spawn(async move {
                        (
                            fh,
                            mount.client.renew_handle(id, &IdempotencyKey::new()).await,
                        )
                    });
                }
                while let Some(result) = jobs.join_next().await {
                    let Ok((fh, result)) = result else { continue };
                    match result {
                        Ok(_) => {
                            retries.remove(&fh);
                            if let Some(f) = lock(&mount.files).entries.get_mut(&fh) {
                                f.renewed = Instant::now();
                            }
                        }
                        Err(error) => {
                            let backoff = retries
                                .get(&fh)
                                .map_or(Duration::from_millis(250), |(_, b)| *b);
                            retries.insert(
                                fh,
                                (
                                    Instant::now() + renewal_delay(&error, backoff),
                                    (backoff * 2).min(
                                        (due / 4).clamp(
                                            Duration::from_millis(250),
                                            Duration::from_secs(5),
                                        ),
                                    ),
                                ),
                            );
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
            Ok(value) => {
                if matches!(class, CallClass::Read) {
                    self.live()?;
                }
                Ok(value)
            }
            Err(error) => Err(self.api_error(error, class, fh).await),
        }
    }
    pub async fn api_error(
        &self,
        error: ClientError,
        class: CallClass,
        fh: Option<u64>,
    ) -> ErrnoName {
        if self.fence.load(Ordering::SeqCst) {
            return ErrnoName::ESTALE;
        }
        // Retrying an unresolved append under a new key can duplicate bytes.
        // Keep the first error as EIO; require a fresh handle for later calls.
        if matches!(class, CallClass::Mutation)
            && error.is_indeterminate()
            && let Some(id) = fh
            && let Some(file) = lock(&self.files).entries.get_mut(&id)
        {
            file.fenced = true;
        }
        if let ClientError::Api(e) = &error {
            tracing::warn!(status=e.status,code=%e.code,trace_id=%e.trace_id,operation_id=?e.operation_id,max_io_bytes=self.cfg.args.max_io_bytes,"native API call failed");
            if e.status == 401 {
                self.fence.store(true, Ordering::SeqCst);
                return ErrnoName::ESTALE;
            }
            let handle_fenced = e.code == "stale_handle"
                || (e.code == "fenced" && matches!(class, CallClass::Read));
            if let Some(id) = fh.filter(|_| handle_fenced)
                && let Some(file) = lock(&self.files).entries.get_mut(&id)
            {
                file.fenced = true;
            }
            if e.code == "fenced" {
                let bound_mutation = self.client.authority().workflow.is_some()
                    && matches!(class, CallClass::Mutation)
                    && e.message != anchorfs_core::error::RESERVATION_EXPIRED;
                if bound_mutation {
                    // Mere lease expiry still permits server reads. Latch before
                    // any await so another callback cannot start a new read.
                    self.fence.store(true, Ordering::SeqCst);
                } else {
                    let probe = self.client.stat(None).await;
                    let dead = matches!(&probe,Err(ClientError::Api(p)) if p.status==401 || p.code=="fenced");
                    if dead {
                        self.fence.store(true, Ordering::SeqCst);
                    }
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
        // Linux can omit FUSE_GETATTR_FH for fstat after the last name is
        // unlinked. Resolve that inode through an existing server handle,
        // never through a detached/replaced path.
        let fh = fh.or_else(|| {
            let server_id = lock(&self.inodes).server_id(ino);
            lock(&self.files)
                .entries
                .iter()
                .find(|(_, file)| file.server_ino == server_id && !file.fenced)
                .map(|(fh, _)| *fh)
        });
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
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
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

fn renewal_delay(error: &ClientError, backoff: Duration) -> Duration {
    match error {
        ClientError::Api(e) => e
            .retry_after_ms
            .map(Duration::from_millis)
            .unwrap_or(backoff)
            .max(backoff),
        _ => backoff,
    }
}
