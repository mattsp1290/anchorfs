//! Linux mount authority, identity checks, and renewal. No server clocks drive timers.
use crate::{
    MountError,
    config::MountConfig,
    errno::{CallClass, ErrnoName, errno_for},
    handles::*,
    inodes::InodeTable,
};
use anchorfs_client::{Client, error::ClientError, retry::IdempotencyKey, wire::*};
use anchorfs_core::{ErrorCode, Name, WorkspacePath};
use std::{
    future::Future,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
mod renewal;
mod startup;

pub(crate) struct Resolved {
    pub path: Option<WorkspacePath>,
    pub attributes: InodeAttributes,
}
pub struct Mount {
    client: Client,
    cfg: MountConfig,
    inodes: Mutex<InodeTable>,
    files: Mutex<OpenFiles>,
    dirs: Mutex<OpenDirs>,
    fence: AtomicBool,
    stopping: AtomicBool,
    rt: tokio::runtime::Handle,
    self_minted: bool,
    changed: Arc<tokio::sync::Notify>,
}
impl Mount {
    pub async fn start(
        cfg: MountConfig,
        rt: tokio::runtime::Handle,
    ) -> Result<Arc<Self>, MountError> {
        let established = startup::connect(&cfg).await?;
        let mount = Arc::new(Self {
            client: established.client,
            cfg,
            inodes: Mutex::new(InodeTable::new(established.root)),
            files: Mutex::new(OpenFiles::default()),
            dirs: Mutex::new(OpenDirs::default()),
            fence: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            rt,
            self_minted: established.self_minted,
            changed: Arc::new(tokio::sync::Notify::new()),
        });
        renewal::spawn(&mount);
        Ok(mount)
    }
    pub fn config(&self) -> &MountConfig {
        &self.cfg
    }
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }
    fn stopped(&self) -> bool {
        self.stopping.load(Ordering::SeqCst) || self.fence.load(Ordering::SeqCst)
    }
    pub(crate) fn forget(&self, ino: u64, n: u64) {
        lock(&self.inodes).forget(ino, n);
    }
    pub(crate) fn inode_number(&self, id: i64) -> Result<u64, ErrnoName> {
        lock(&self.inodes).ino(id)
    }
    pub(crate) fn child_inode(&self, id: i64) -> Result<u64, ErrnoName> {
        lock(&self.inodes).child_ino(id)
    }
    pub(crate) fn parent(&self, ino: u64) -> Result<u64, ErrnoName> {
        lock(&self.inodes).parent(ino)
    }
    pub(crate) fn record(
        &self,
        parent: u64,
        name: Name,
        attrs: &InodeAttributes,
    ) -> Result<(), ErrnoName> {
        lock(&self.inodes).record_name(attrs.inode_id, parent, name)?;
        Ok(())
    }
    pub(crate) fn unlink_name(&self, parent: u64, name: &Name) {
        lock(&self.inodes).unlink_name(parent, name);
    }
    pub(crate) fn rename_names(&self, parent: u64, name: &Name, newparent: u64, newname: Name) {
        lock(&self.inodes).rename(parent, name, newparent, newname);
    }
    pub(crate) fn remember_file(&self, grant: &HandleGrant) -> u64 {
        let fh = lock(&self.files).insert(OpenFile {
            handle_id: grant.handle_id,
            server_ino: grant.attributes.inode_id,
            fenced: false,
            renewed: Instant::now(),
        });
        self.changed.notify_one();
        fh
    }
    pub(crate) fn take_file(&self, fh: u64) -> Option<OpenFile> {
        let file = lock(&self.files).entries.remove(&fh);
        self.changed.notify_one();
        file
    }
    pub(crate) fn remember_directory(&self, entries: Vec<DirEntrySnapshot>) -> u64 {
        lock(&self.dirs).insert(entries)
    }
    pub(crate) fn release_directory(&self, fh: u64) {
        lock(&self.dirs).entries.remove(&fh);
    }
    pub(crate) fn with_directory<T>(
        &self,
        fh: u64,
        f: impl FnOnce(&[DirEntrySnapshot]) -> T,
    ) -> Result<T, ErrnoName> {
        let dirs = lock(&self.dirs);
        let entries = dirs.entries.get(&fh).ok_or(ErrnoName::ESTALE)?;
        Ok(f(entries))
    }
    pub(crate) async fn close_quietly(&self, handle: uuid::Uuid) {
        if let Err(error) = self
            .client
            .close_handle(handle, &IdempotencyKey::new())
            .await
        {
            self.api_error(error, CallClass::Mutation, None).await;
        }
    }
    pub(crate) fn block<T>(&self, future: impl Future<Output = T>) -> T {
        self.rt.block_on(future)
    }
    pub(crate) fn live(&self) -> Result<(), ErrnoName> {
        if self.fence.load(Ordering::SeqCst) {
            Err(ErrnoName::ESTALE)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn api<T>(
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
    pub(crate) async fn api_error(
        &self,
        error: ClientError,
        class: CallClass,
        fh: Option<u64>,
    ) -> ErrnoName {
        if let ClientError::Api(e) = &error {
            tracing::warn!(
                status = e.status, code = %e.code, trace_id = %e.trace_id,
                operation_id = ?e.operation_id, max_io_bytes = self.cfg.max_io,
                "native API call failed"
            );
        } else {
            tracing::warn!(code = error.code(), "native API call failed");
        }
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
            if e.status == 401 {
                self.latch_fence("unauthenticated");
                return ErrnoName::ESTALE;
            }
            let handle_fenced = e.code.kind() == Some(ErrorCode::StaleHandle)
                || (e.code.kind() == Some(ErrorCode::Fenced) && matches!(class, CallClass::Read));
            if let Some(id) = fh.filter(|_| handle_fenced)
                && let Some(file) = lock(&self.files).entries.get_mut(&id)
            {
                file.fenced = true;
            }
            if e.code.kind() == Some(ErrorCode::Fenced) {
                let bound_mutation = self.client.authority().workflow.is_some()
                    && matches!(class, CallClass::Mutation)
                    && e.message != anchorfs_core::error::RESERVATION_EXPIRED;
                if bound_mutation {
                    // Mere lease expiry still permits server reads. Latch before
                    // any await so another callback cannot start a new read.
                    self.latch_fence("fenced_mutation");
                } else {
                    let probe = self.client.stat(None).await;
                    let dead = matches!(
                        &probe,
                        Err(ClientError::Api(p)) if p.status == 401 || p.code.kind() == Some(ErrorCode::Fenced)
                    );
                    if dead {
                        self.latch_fence("probe");
                    }
                }
            }
        }
        errno_for(&error, class)
    }
    /// Latches the mount fence and emits the lifecycle event the cluster proof depends on.
    fn latch_fence(&self, reason: &'static str) {
        self.fence.store(true, Ordering::SeqCst);
        tracing::warn!(reason, "mount fenced");
    }
    pub(crate) fn file(&self, fh: u64) -> Result<OpenFile, ErrnoName> {
        self.live()?;
        lock(&self.files)
            .entries
            .get(&fh)
            .filter(|f| !f.fenced)
            .cloned()
            .ok_or(ErrnoName::ESTALE)
    }
    pub(crate) async fn resolve(&self, ino: u64) -> Result<Resolved, ErrnoName> {
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
        Ok(Resolved { path, attributes })
    }
    pub(crate) async fn checked_path(&self, ino: u64) -> Result<Option<WorkspacePath>, ErrnoName> {
        Ok(self.resolve(ino).await?.path)
    }
    pub(crate) fn identity(&self, ino: u64, server: i64) -> Result<(), ErrnoName> {
        let mut table = lock(&self.inodes);
        if table.server_id(ino) != server {
            table.detach(ino);
            Err(ErrnoName::ESTALE)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn child_path(&self, parent: u64, name: &Name) -> Result<String, ErrnoName> {
        let parent = self.checked_path(parent).await?;
        let path = parent.map_or_else(
            || name.as_str().to_owned(),
            |p| format!("{p}/{}", name.as_str()),
        );
        WorkspacePath::parse(&path).map_err(|_| ErrnoName::ENAMETOOLONG)?;
        Ok(path)
    }
    pub(crate) async fn attributes(
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
            self.resolve(ino).await?.attributes
        };
        self.identity(ino, attributes.inode_id)?;
        Ok(attributes)
    }
    pub async fn shutdown(&self) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        self.changed.notify_one();
        let files = std::mem::take(&mut lock(&self.files).entries);
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            for (_, file) in files {
                self.close_quietly(file.handle_id).await;
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
