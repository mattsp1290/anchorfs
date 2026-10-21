use crate::{
    errno::{CallClass, ErrnoName},
    handles::OpenFile,
    runtime::{Mount, lock},
};
use anchorfs_client::{retry::IdempotencyKey, wire::*};
use anchorfs_core::{Name, WorkspacePath};
use fuser::{FileAttr, FileType, INodeNo, TimeOrNow};
use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};

pub fn file_type(kind: &InodeKind) -> FileType {
    match kind {
        InodeKind::File => FileType::RegularFile,
        InodeKind::Directory => FileType::Directory,
        InodeKind::Symlink => FileType::Symlink,
    }
}
pub fn attr(m: &Mount, a: &InodeAttributes) -> Result<FileAttr, ErrnoName> {
    Ok(FileAttr {
        ino: INodeNo(lock(&m.inodes).ino(a.inode_id)?),
        size: a.size,
        blocks: a.size.div_ceil(512),
        atime: a.atime.into(),
        mtime: a.mtime.into(),
        ctime: a.ctime.into(),
        crtime: a.ctime.into(),
        kind: file_type(&a.kind),
        perm: a.mode as u16,
        nlink: a.nlink,
        uid: a.uid,
        gid: a.gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    })
}
pub fn record(
    m: &Mount,
    parent: u64,
    name: Name,
    a: &InodeAttributes,
) -> Result<FileAttr, ErrnoName> {
    lock(&m.inodes).record_name(a.inode_id, parent, name, a.kind.clone())?;
    attr(m, a)
}
pub fn mode(mode: u32) -> Result<u32, ErrnoName> {
    if mode & 0o7000 != 0 {
        Err(ErrnoName::EPERM)
    } else {
        Ok(mode & 0o777)
    }
}
pub fn open_body(path: String, flags: i32, create: bool, mode: Option<u32>) -> OpenHandle {
    OpenHandle {
        path,
        read: flags & libc::O_ACCMODE != libc::O_WRONLY,
        write: flags & libc::O_ACCMODE != libc::O_RDONLY,
        create,
        exclusive: flags & libc::O_EXCL != 0,
        truncate: flags & libc::O_TRUNC != 0,
        mode,
    }
}
pub fn save_file(m: &Mount, h: &HandleGrant, flags: i32) -> u64 {
    lock(&m.files).insert(OpenFile {
        handle_id: h.handle_id,
        server_ino: h.attributes.inode_id,
        read: h.access_read,
        write: h.access_write,
        append: flags & libc::O_APPEND != 0,
        fenced: false,
        renewed: Instant::now(),
    })
}
pub async fn open_checked(m: &Mount, ino: u64, flags: i32) -> Result<HandleGrant, ErrnoName> {
    let path = m.checked_path(ino).await?.ok_or(ErrnoName::EISDIR)?;
    let h = m
        .api(
            m.client
                .open_handle(
                    &open_body(path.to_string(), flags, false, None),
                    &IdempotencyKey::new(),
                )
                .await,
            CallClass::Mutation,
            None,
        )
        .await?;
    if let Err(e) = m.identity(ino, h.attributes.inode_id) {
        let _ = m
            .client
            .close_handle(h.handle_id, &IdempotencyKey::new())
            .await;
        return Err(e);
    }
    Ok(h)
}
#[derive(Default)]
pub struct Attributes {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub atime: Option<TimeOrNow>,
    pub mtime: Option<TimeOrNow>,
    pub fh: Option<u64>,
}
pub async fn setattr(m: &Mount, ino: u64, change: Attributes) -> Result<FileAttr, ErrnoName> {
    // Reject forbidden mode bits before a size change can publish.
    let mode = change.mode.map(mode).transpose()?;
    let before = m.attributes(ino, change.fh).await?;
    if let Some(size) = change.size {
        if let Some(fh) = change.fh {
            let file = m.file(fh)?;
            m.api(
                m.client
                    .truncate(file.handle_id, size, &IdempotencyKey::new())
                    .await,
                CallClass::Mutation,
                Some(fh),
            )
            .await?;
        } else {
            let h = open_checked(m, ino, libc::O_WRONLY).await?;
            let result = m
                .api(
                    m.client
                        .truncate(h.handle_id, size, &IdempotencyKey::new())
                        .await,
                    CallClass::Mutation,
                    None,
                )
                .await;
            let _ = m
                .client
                .close_handle(h.handle_id, &IdempotencyKey::new())
                .await;
            result?;
        }
    }
    let attributes = SetAttributes {
        path: None,
        mode,
        uid: change.uid.filter(|v| *v != before.uid),
        gid: change.gid.filter(|v| *v != before.gid),
        atime: change.atime.map(timestamp).transpose()?,
        mtime: change.mtime.map(timestamp).transpose()?,
    };
    if attributes.mode.is_some()
        || attributes.uid.is_some()
        || attributes.gid.is_some()
        || attributes.atime.is_some()
        || attributes.mtime.is_some()
    {
        let path = m.checked_path(ino).await?;
        let attributes = SetAttributes {
            path: path.map(|p| p.to_string()),
            ..attributes
        };
        let result = m
            .api(
                m.client
                    .set_attributes(&attributes, &IdempotencyKey::new())
                    .await,
                CallClass::Mutation,
                change.fh,
            )
            .await?;
        if let Some(id) = result.inode_id {
            m.identity(ino, id)?;
        }
    }
    attr(m, &m.attributes(ino, change.fh).await?)
}
fn timestamp(value: TimeOrNow) -> Result<String, ErrnoName> {
    let instant: time::OffsetDateTime = match value {
        TimeOrNow::Now => SystemTime::now(),
        TimeOrNow::SpecificTime(t) => t,
    }
    .into();
    instant
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| ErrnoName::EINVAL)
}
pub async fn remove(m: &Mount, parent: u64, name: Name, directory: bool) -> Result<(), ErrnoName> {
    let path = m.child_path(parent, &name).await?;
    let key = IdempotencyKey::new();
    let result = if directory {
        m.client.remove_directory(&path, &key).await
    } else {
        m.client.unlink(&path, &key).await
    };
    m.api(result, CallClass::Mutation, None).await?;
    lock(&m.inodes).unlink_name(parent, &name);
    Ok(())
}
pub async fn stat_child(
    m: &Arc<Mount>,
    path: &str,
    parent: u64,
    name: Name,
    expected: Option<i64>,
) -> Result<FileAttr, ErrnoName> {
    let a = m
        .api(m.client.stat(Some(path)).await, CallClass::Read, None)
        .await?;
    if expected.is_some_and(|id| id != a.inode_id) {
        return Err(ErrnoName::ESTALE);
    }
    record(m, parent, name, &a)
}
pub fn path_str(path: &Option<WorkspacePath>) -> Option<&str> {
    path.as_ref().map(WorkspacePath::as_str)
}
