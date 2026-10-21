mod translation;
use crate::{
    MountError,
    config::MountConfig,
    errno::{CallClass, ErrnoName},
    handles::{DirEntrySnapshot, write_slices},
    inodes::name,
    profile::MountProfile,
    runtime::{Mount, lock},
};
use anchorfs_client::{retry::IdempotencyKey, wire::InodeKind};
use fuser::*;
use std::{
    ffi::OsStr,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};
use translation::*;

pub struct AnchorFs {
    pub mount: Arc<Mount>,
}
macro_rules! empty_reply {
    ($reply:expr,$result:expr) => {
        match $result {
            Ok(()) => $reply.ok(),
            Err(e) => $reply.error(Errno::from(e)),
        }
    };
}
macro_rules! attr_reply {
    ($reply:expr,$result:expr) => {
        match $result {
            Ok(a) => $reply.attr(&Duration::ZERO, &a),
            Err(e) => $reply.error(Errno::from(e)),
        }
    };
}
macro_rules! entry_reply {
    ($reply:expr,$result:expr) => {
        match $result {
            Ok(a) => $reply.entry(&Duration::ZERO, &a, Generation(1)),
            Err(e) => $reply.error(Errno::from(e)),
        }
    };
}
impl Filesystem for AnchorFs {
    fn init(&mut self, _: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        config
            .add_capabilities(InitFlags::FUSE_POSIX_LOCKS | InitFlags::FUSE_FLOCK_LOCKS)
            .map_err(|_| {
                std::io::Error::other(
                    "kernel cannot forward locks; local lock emulation is forbidden",
                )
            })?;
        config
            .set_max_write(self.mount.cfg.args.max_io_bytes)
            .map_err(|_| std::io::Error::other("kernel refused max_write"))?;
        tracing::info!(capabilities=?config.capabilities(),max_io_bytes=self.mount.cfg.args.max_io_bytes,"strict mount initialized");
        Ok(())
    }
    fn destroy(&mut self) {
        self.mount.block(self.mount.shutdown());
    }
    fn lookup(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEntry) {
        entry_reply!(
            reply,
            self.mount.block(async {
                let n = name(n)?;
                let path = self.mount.child_path(parent.0, &n).await?;
                stat_child(&self.mount, &path, parent.0, n, None).await
            })
        );
    }
    fn forget(&self, _: &Request, ino: INodeNo, count: u64) {
        lock(&self.mount.inodes).forget(ino.0, count);
    }
    fn getattr(&self, _: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        attr_reply!(
            reply,
            self.mount.block(async {
                attr(
                    &self.mount,
                    &self.mount.attributes(ino.0, fh.map(|v| v.0)).await?,
                )
            })
        );
    }
    fn setattr(
        &self,
        _: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _: Option<SystemTime>,
        fh: Option<FileHandle>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        attr_reply!(
            reply,
            self.mount.block(setattr(
                &self.mount,
                ino.0,
                Attributes {
                    mode,
                    uid,
                    gid,
                    size,
                    atime,
                    mtime,
                    fh: fh.map(|h| h.0)
                }
            ))
        );
    }
    fn readlink(&self, _: &Request, ino: INodeNo, reply: ReplyData) {
        let result = self.mount.block(async {
            let path = self
                .mount
                .checked_path(ino.0)
                .await?
                .ok_or(ErrnoName::EINVAL)?;
            let result = self
                .mount
                .api(
                    self.mount.client.read_symlink(path.as_str()).await,
                    CallClass::Read,
                    None,
                )
                .await?;
            self.mount.checked_path(ino.0).await?;
            Ok::<_, ErrnoName>(result.target)
        });
        match result {
            Ok(target) => reply.data(target.as_bytes()),
            Err(e) => reply.error(e.into()),
        }
    }
    fn mkdir(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        bits: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        entry_reply!(
            reply,
            self.mount.block(async {
                let bits = mode(bits)? & !umask;
                let n = name(n)?;
                let path = self.mount.child_path(parent.0, &n).await?;
                let result = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .create_directory(&path, bits, &IdempotencyKey::new())
                            .await,
                        CallClass::Mutation,
                        None,
                    )
                    .await?;
                stat_child(&self.mount, &path, parent.0, n, result.inode_id).await
            })
        );
    }
    fn mknod(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        bits: u32,
        umask: u32,
        _: u32,
        reply: ReplyEntry,
    ) {
        entry_reply!(
            reply,
            self.mount.block(async {
                self.mount.live()?;
                if bits & libc::S_IFMT != libc::S_IFREG {
                    return Err(ErrnoName::EPERM);
                }
                let bits = mode(bits)? & !umask;
                let n = name(n)?;
                let path = self.mount.child_path(parent.0, &n).await?;
                let h = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .open_handle(
                                &open_body(path, libc::O_WRONLY | libc::O_EXCL, true, Some(bits)),
                                &IdempotencyKey::new(),
                            )
                            .await,
                        CallClass::Mutation,
                        None,
                    )
                    .await?;
                let close = self
                    .mount
                    .client
                    .close_handle(h.handle_id, &IdempotencyKey::new())
                    .await;
                self.mount.api(close, CallClass::Mutation, None).await?;
                record(&self.mount, parent.0, n, &h.attributes)
            })
        );
    }
    fn unlink(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEmpty) {
        empty_reply!(
            reply,
            self.mount
                .block(async { remove(&self.mount, parent.0, name(n)?, false).await })
        );
    }
    fn rmdir(&self, _: &Request, parent: INodeNo, n: &OsStr, reply: ReplyEmpty) {
        empty_reply!(
            reply,
            self.mount
                .block(async { remove(&self.mount, parent.0, name(n)?, true).await })
        );
    }
    fn symlink(&self, _: &Request, parent: INodeNo, n: &OsStr, target: &Path, reply: ReplyEntry) {
        entry_reply!(
            reply,
            self.mount.block(async {
                let target = target
                    .to_str()
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .ok_or(ErrnoName::EINVAL)?;
                let n = name(n)?;
                let path = self.mount.child_path(parent.0, &n).await?;
                let result = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .create_symlink(&path, target, &IdempotencyKey::new())
                            .await,
                        CallClass::Mutation,
                        None,
                    )
                    .await?;
                stat_child(&self.mount, &path, parent.0, n, result.inode_id).await
            })
        );
    }
    fn rename(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        newparent: INodeNo,
        newn: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        empty_reply!(
            reply,
            self.mount.block(async {
                self.mount.live()?;
                if !flags.is_empty() {
                    return Err(ErrnoName::EINVAL);
                }
                let (n, newn) = (name(n)?, name(newn)?);
                let from = self.mount.child_path(parent.0, &n).await?;
                let to = self.mount.child_path(newparent.0, &newn).await?;
                let before = self
                    .mount
                    .api(
                        self.mount.client.stat(Some(&from)).await,
                        CallClass::Read,
                        None,
                    )
                    .await?;
                let result = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .rename(&from, &to, &IdempotencyKey::new())
                            .await,
                        CallClass::Mutation,
                        None,
                    )
                    .await?;
                if result.inode_id != Some(before.inode_id) {
                    return Err(ErrnoName::ESTALE);
                }
                lock(&self.mount.inodes).rename(parent.0, &n, newparent.0, newn);
                Ok(())
            })
        );
    }
    fn link(&self, _: &Request, ino: INodeNo, parent: INodeNo, n: &OsStr, reply: ReplyEntry) {
        entry_reply!(
            reply,
            self.mount.block(async {
                let from = self
                    .mount
                    .checked_path(ino.0)
                    .await?
                    .ok_or(ErrnoName::EPERM)?;
                let n = name(n)?;
                let to = self.mount.child_path(parent.0, &n).await?;
                let result = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .create_link(from.as_str(), &to, &IdempotencyKey::new())
                            .await,
                        CallClass::Mutation,
                        None,
                    )
                    .await?;
                self.mount
                    .identity(ino.0, result.inode_id.ok_or(ErrnoName::EIO)?)?;
                stat_child(&self.mount, &to, parent.0, n, result.inode_id).await
            })
        );
    }
    fn open(&self, _: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        match self.mount.block(open_checked(&self.mount, ino.0, flags.0)) {
            Ok(h) => reply.opened(
                FileHandle(save_file(&self.mount, &h, flags.0)),
                FopenFlags::FOPEN_DIRECT_IO,
            ),
            Err(e) => reply.error(e.into()),
        }
    }
    fn create(
        &self,
        _: &Request,
        parent: INodeNo,
        n: &OsStr,
        bits: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let result = self.mount.block(async {
            let bits = mode(bits)? & !umask;
            let n = name(n)?;
            let path = self.mount.child_path(parent.0, &n).await?;
            let h = self
                .mount
                .api(
                    self.mount
                        .client
                        .open_handle(
                            &open_body(path, flags, true, Some(bits)),
                            &IdempotencyKey::new(),
                        )
                        .await,
                    CallClass::Mutation,
                    None,
                )
                .await?;
            let a = match record(&self.mount, parent.0, n, &h.attributes) {
                Ok(a) => a,
                Err(e) => {
                    let _ = self
                        .mount
                        .client
                        .close_handle(h.handle_id, &IdempotencyKey::new())
                        .await;
                    return Err(e);
                }
            };
            Ok::<_, ErrnoName>((a, save_file(&self.mount, &h, flags)))
        });
        match result {
            Ok((a, fh)) => reply.created(
                &Duration::ZERO,
                &a,
                Generation(1),
                FileHandle(fh),
                FopenFlags::FOPEN_DIRECT_IO,
            ),
            Err(e) => reply.error(e.into()),
        }
    }
    fn read(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let result = self.mount.block(async {
            self.mount.file(fh.0)?;
            let mut bytes = Vec::new();
            while bytes.len() < size as usize {
                let file = match self.mount.file(fh.0) {
                    Ok(file) => file,
                    Err(error) => {
                        if bytes.is_empty() {
                            return Err(error);
                        }
                        break;
                    }
                };
                let n = (size as usize - bytes.len()).min(self.mount.cfg.args.max_io_bytes as usize)
                    as u32;
                let at = offset
                    .checked_add(bytes.len() as u64)
                    .ok_or(ErrnoName::EFBIG)?;
                let result = self
                    .mount
                    .api(
                        self.mount.client.read_range(file.handle_id, at, n).await,
                        CallClass::Read,
                        Some(fh.0),
                    )
                    .await;
                match result {
                    Ok((part, _)) => {
                        let short = part.len() < n as usize;
                        bytes.extend_from_slice(&part);
                        if short {
                            break;
                        }
                    }
                    Err(e) => {
                        if bytes.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                }
            }
            Ok::<_, ErrnoName>(bytes)
        });
        match result {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(e.into()),
        }
    }
    fn write(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _: WriteFlags,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let result = self.mount.block(async {
            let file = self.mount.file(fh.0)?;
            let (done, error) = write_slices(
                data,
                offset,
                self.mount.cfg.args.max_io_bytes as usize,
                |at, bytes, key| {
                    let file = &file;
                    async move {
                        self.mount.file(fh.0).map_err(|_| {
                            anchorfs_client::error::ClientError::Api(
                                anchorfs_client::error::ApiFailure {
                                    status: 410,
                                    code: "stale_handle".into(),
                                    message: "local mount or handle fenced".into(),
                                    trace_id: String::new(),
                                    operation_id: None,
                                    retry_after_ms: None,
                                },
                            )
                        })?;
                        let result = if file.append {
                            self.mount.client.append(file.handle_id, bytes, &key).await
                        } else {
                            self.mount
                                .client
                                .write_at(file.handle_id, at, bytes, &key)
                                .await
                        };
                        if let Err(error) = &result {
                            // latch before another slice or callback can start
                            let e = match error {
                                anchorfs_client::error::ClientError::Api(e) => {
                                    anchorfs_client::error::ClientError::Api(e.clone())
                                }
                                _ => anchorfs_client::error::ClientError::Transport(
                                    "write failed".into(),
                                ),
                            };
                            self.mount
                                .api_error(e, CallClass::Mutation, Some(fh.0))
                                .await;
                        }
                        result
                    }
                },
            )
            .await;
            if done > 0 {
                Ok(done as u32)
            } else if let Some(e) = error {
                Err(e)
            } else {
                Ok(0)
            }
        });
        match result {
            Ok(n) => reply.written(n),
            Err(e) => reply.error(e.into()),
        }
    }
    fn flush(&self, _: &Request, _: INodeNo, _: FileHandle, _: LockOwner, reply: ReplyEmpty) {
        // Synchronous writes expose their own errors; close must not invent one.
        reply.ok();
    }
    fn fsync(&self, _: &Request, _: INodeNo, fh: FileHandle, _: bool, reply: ReplyEmpty) {
        empty_reply!(
            reply,
            self.mount.block(async {
                let file = self.mount.file(fh.0)?;
                self.mount
                    .api(
                        self.mount.client.fsync(file.handle_id).await,
                        CallClass::Read,
                        Some(fh.0),
                    )
                    .await?;
                Ok::<(), ErrnoName>(())
            })
        );
    }
    fn release(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        let file = lock(&self.mount.files).entries.remove(&fh.0);
        if let Some(file) = file {
            let result = self.mount.block(
                self.mount
                    .client
                    .close_handle(file.handle_id, &IdempotencyKey::new()),
            );
            if let Err(error) = result {
                tracing::warn!(code = error.code(), "close handle failed");
            }
        }
        reply.ok();
    }
    fn opendir(&self, _: &Request, ino: INodeNo, _: OpenFlags, reply: ReplyOpen) {
        let result = self.mount.block(async {
            let path = self.mount.checked_path(ino.0).await?;
            let parent = lock(&self.mount.inodes).parent(ino.0)?;
            let mut snapshot = vec![
                DirEntrySnapshot {
                    ino: ino.0,
                    name: ".".into(),
                    kind: InodeKind::Directory,
                },
                DirEntrySnapshot {
                    ino: parent,
                    name: "..".into(),
                    kind: InodeKind::Directory,
                },
            ];
            let mut cursor = None;
            loop {
                self.mount.live()?;
                let page = self
                    .mount
                    .api(
                        self.mount
                            .client
                            .list_directory(path_str(&path), cursor.as_deref(), 1000)
                            .await,
                        CallClass::Read,
                        None,
                    )
                    .await?;
                for e in page.entries {
                    snapshot.push(DirEntrySnapshot {
                        ino: lock(&self.mount.inodes).child_ino(e.attributes.inode_id)?,
                        name: e.name,
                        kind: e.attributes.kind,
                    });
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            Ok::<_, ErrnoName>(lock(&self.mount.dirs).insert(snapshot))
        });
        match result {
            Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::empty()),
            Err(e) => reply.error(e.into()),
        }
    }
    fn readdir(
        &self,
        _: &Request,
        _: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        if let Err(e) = self.mount.live() {
            reply.error(e.into());
            return;
        }
        let dirs = lock(&self.mount.dirs);
        let Some(entries) = dirs.entries.get(&fh.0) else {
            reply.error(Errno::ESTALE);
            return;
        };
        for (i, e) in entries
            .iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        {
            if reply.add(INodeNo(e.ino), (i + 1) as u64, file_type(&e.kind), &e.name) {
                break;
            }
        }
        reply.ok();
    }
    fn releasedir(&self, _: &Request, _: INodeNo, fh: FileHandle, _: OpenFlags, reply: ReplyEmpty) {
        lock(&self.mount.dirs).entries.remove(&fh.0);
        reply.ok();
    }
    fn fsyncdir(&self, _: &Request, _: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        empty_reply!(reply, self.mount.live());
    }
    fn statfs(&self, _: &Request, _: INodeNo, reply: ReplyStatfs) {
        reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096);
    }
    fn getlk(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: LockOwner,
        start: u64,
        end: u64,
        typ: i32,
        _: u32,
        reply: ReplyLock,
    ) {
        if self.mount.live().is_err() {
            reply.error(Errno::ESTALE);
        } else if typ == libc::F_UNLCK {
            reply.locked(start, end, typ, 0);
        } else {
            reply.error(Errno::ENOLCK);
        }
    }
    fn setlk(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: LockOwner,
        _: u64,
        _: u64,
        typ: i32,
        _: u32,
        _: bool,
        reply: ReplyEmpty,
    ) {
        if let Err(e) = self.mount.live() {
            reply.error(e.into());
        } else if typ == libc::F_UNLCK {
            reply.ok();
        } else {
            reply.error(Errno::ENOLCK);
        }
    }
    fn readdirplus(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: u64,
        reply: ReplyDirectoryPlus,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn access(&self, _: &Request, _: INodeNo, _: AccessFlags, reply: ReplyEmpty) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn setxattr(
        &self,
        _: &Request,
        _: INodeNo,
        _: &OsStr,
        _: &[u8],
        _: i32,
        _: u32,
        reply: ReplyEmpty,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn getxattr(&self, _: &Request, _: INodeNo, _: &OsStr, _: u32, reply: ReplyXattr) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn listxattr(&self, _: &Request, _: INodeNo, _: u32, reply: ReplyXattr) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn removexattr(&self, _: &Request, _: INodeNo, _: &OsStr, reply: ReplyEmpty) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn ioctl(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: IoctlFlags,
        _: u32,
        _: &[u8],
        _: u32,
        reply: ReplyIoctl,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn poll(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: PollNotifier,
        _: PollEvents,
        _: PollFlags,
        reply: ReplyPoll,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn fallocate(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: u64,
        _: u64,
        _: i32,
        reply: ReplyEmpty,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn lseek(&self, _: &Request, _: INodeNo, _: FileHandle, _: i64, _: i32, reply: ReplyLseek) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn copy_file_range(
        &self,
        _: &Request,
        _: INodeNo,
        _: FileHandle,
        _: u64,
        _: INodeNo,
        _: FileHandle,
        _: u64,
        _: u64,
        _: CopyFileRangeFlags,
        reply: ReplyWrite,
    ) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
    fn bmap(&self, _: &Request, _: INodeNo, _: u32, _: u64, reply: ReplyBmap) {
        reply.error(if self.mount.live().is_err() {
            Errno::ESTALE
        } else {
            Errno::ENOSYS
        });
    }
}
pub fn platform_supported() -> bool {
    true
}
pub fn mount_options(profile: &MountProfile) -> Result<Vec<String>, MountError> {
    if profile != &MountProfile::strict() {
        return Err(MountError::UnsupportedProfile);
    }
    Ok([
        "default_permissions",
        "nosuid",
        "nodev",
        "noatime",
        "fsname=anchorfs",
        "subtype=anchorfs",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
}
pub fn fuse_config(cfg: &MountConfig) -> Config {
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::DefaultPermissions,
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::NoAtime,
        MountOption::FSName("anchorfs".into()),
        MountOption::Subtype("anchorfs".into()),
    ];
    config.acl = if cfg.args.allow_other {
        SessionACL::All
    } else {
        SessionACL::Owner
    };
    config.n_threads = Some(cfg.args.fuse_threads);
    config.clone_fd = true;
    config
}
pub fn spawn(mount: Arc<Mount>) -> Result<BackgroundSession, MountError> {
    let config = fuse_config(&mount.cfg);
    let path = mount.cfg.args.mountpoint.clone();
    fuser::spawn_mount(AnchorFs { mount }, path, &config).map_err(|e| MountError::Mount {
        kind: format!("{:?}", e.kind()),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct Args {
        #[command(flatten)]
        mount: crate::config::MountArgs,
    }
    #[test]
    fn rejects_relaxed_profile() {
        let mut p = MountProfile::strict();
        p.writeback_cache = true;
        assert_eq!(mount_options(&p), Err(MountError::UnsupportedProfile));
    }
    #[test]
    fn fuse_config_and_option_rendering() {
        let args = Args::parse_from([
            "test",
            "/tmp/synthetic",
            "--endpoint",
            "http://localhost:1234",
            "--workspace",
            "00000000-0000-4000-8000-000000000001",
            "--allow-plaintext",
        ])
        .mount;
        let bearer = format!("{}.{}", uuid::Uuid::new_v4(), "ab".repeat(32));
        let mut cfg = args.with_secrets(&bearer, None).unwrap();
        assert_eq!(fuse_config(&cfg).acl, SessionACL::Owner);
        cfg.args.allow_other = true;
        assert_eq!(fuse_config(&cfg).acl, SessionACL::All);
        assert_eq!(
            mount_options(&MountProfile::strict()).unwrap(),
            vec![
                "default_permissions",
                "nosuid",
                "nodev",
                "noatime",
                "fsname=anchorfs",
                "subtype=anchorfs"
            ]
        );
    }
}
