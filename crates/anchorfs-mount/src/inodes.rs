use crate::errno::ErrnoName;
use anchorfs_client::wire::InodeKind;
use anchorfs_core::{Name, PathError, WorkspacePath};
use std::{collections::BTreeMap, ffi::OsStr};

#[derive(Debug)]
pub struct Node {
    pub names: Vec<(u64, Name)>,
    pub kind: InodeKind,
    pub nlookup: u64,
    pub detached: bool,
}
pub struct InodeTable {
    root: i64,
    nodes: BTreeMap<u64, Node>,
}
impl InodeTable {
    pub fn new(root: i64) -> Self {
        Self {
            root,
            nodes: BTreeMap::from([(
                1,
                Node {
                    names: vec![],
                    kind: InodeKind::Directory,
                    nlookup: 1,
                    detached: false,
                },
            )]),
        }
    }
    pub fn ino(&self, server_id: i64) -> Result<u64, ErrnoName> {
        if server_id == self.root {
            Ok(1)
        } else if server_id <= 1 {
            Err(ErrnoName::EIO)
        } else {
            Ok(server_id as u64)
        }
    }
    pub fn child_ino(&self, server_id: i64) -> Result<u64, ErrnoName> {
        let ino = self.ino(server_id)?;
        if ino == 1 {
            Err(ErrnoName::EIO)
        } else {
            Ok(ino)
        }
    }
    pub fn server_id(&self, ino: u64) -> i64 {
        if ino == 1 { self.root } else { ino as i64 }
    }
    pub fn record_name(
        &mut self,
        server_id: i64,
        parent: u64,
        name: Name,
        kind: InodeKind,
    ) -> Result<u64, ErrnoName> {
        let ino = self.child_ino(server_id)?;
        // A fresh lookup can observe a different inode at a known name.
        for (&other, node) in &mut self.nodes {
            if other != ino {
                node.names.retain(|(p, n)| *p != parent || *n != name);
                if other != 1 && node.names.is_empty() {
                    node.detached = true;
                }
            }
        }
        let node = self.nodes.entry(ino).or_insert(Node {
            names: vec![],
            kind,
            nlookup: 0,
            detached: false,
        });
        if !node.names.contains(&(parent, name.clone())) {
            node.names.push((parent, name));
        }
        node.nlookup = node.nlookup.saturating_add(1);
        node.detached = false;
        Ok(ino)
    }
    pub fn path(&self, ino: u64) -> Result<Option<WorkspacePath>, ErrnoName> {
        if ino == 1 {
            return Ok(None);
        }
        let mut parts = Vec::new();
        let mut next = ino;
        let mut length = 0usize;
        while next != 1 {
            let node = self
                .nodes
                .get(&next)
                .filter(|n| !n.detached)
                .ok_or(ErrnoName::ESTALE)?;
            let (parent, name) = node.names.first().ok_or(ErrnoName::ESTALE)?;
            length += name.as_str().len() + usize::from(!parts.is_empty());
            if length > 4096 || parts.len() > 4096 {
                return Err(ErrnoName::ENAMETOOLONG);
            }
            parts.push(name.as_str());
            next = *parent;
        }
        parts.reverse();
        WorkspacePath::parse(parts.join("/"))
            .map(Some)
            .map_err(path_errno)
    }
    pub fn parent(&self, ino: u64) -> Result<u64, ErrnoName> {
        if ino == 1 {
            Ok(1)
        } else {
            self.nodes
                .get(&ino)
                .and_then(|n| n.names.first())
                .map(|p| p.0)
                .ok_or(ErrnoName::ESTALE)
        }
    }
    pub fn detach(&mut self, ino: u64) {
        if let Some(n) = self.nodes.get_mut(&ino) {
            n.detached = true;
        }
    }
    pub fn unlink_name(&mut self, parent: u64, name: &Name) {
        for (&ino, node) in &mut self.nodes {
            node.names.retain(|(p, n)| *p != parent || n != name);
            if ino != 1 && node.names.is_empty() {
                node.detached = true;
            }
        }
    }
    pub fn rename(&mut self, parent: u64, name: &Name, newparent: u64, newname: Name) {
        let source = self
            .nodes
            .iter()
            .find(|(_, n)| n.names.contains(&(parent, name.clone())))
            .map(|(id, _)| *id);
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.names.contains(&(newparent, newname.clone())))
            .map(|(id, _)| *id);
        if source.is_some() && source == target {
            return;
        } // POSIX same-inode rename is a no-op.
        self.unlink_name(newparent, &newname);
        if let Some(node) = source.and_then(|id| self.nodes.get_mut(&id)) {
            node.names.retain(|(p, n)| *p != parent || n != name);
            node.names.push((newparent, newname));
            node.detached = false;
        }
    }
    pub fn forget(&mut self, ino: u64, count: u64) {
        if ino == 1 {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&ino) {
            node.nlookup = node.nlookup.saturating_sub(count);
            if node.nlookup == 0 {
                self.nodes.remove(&ino);
            }
        }
    }
}
pub fn name(value: &OsStr) -> Result<Name, ErrnoName> {
    Name::parse(value.to_str().ok_or(ErrnoName::EINVAL)?).map_err(path_errno)
}
fn path_errno(error: PathError) -> ErrnoName {
    if error == PathError::TooLong {
        ErrnoName::ENAMETOOLONG
    } else {
        ErrnoName::EINVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(s: &str) -> Name {
        Name::parse(s).unwrap()
    }
    #[test]
    fn root_mapping_rename_descendants_and_forget() {
        let mut t = InodeTable::new(42);
        assert_eq!(t.ino(42), Ok(1));
        assert_eq!(t.server_id(1), 42);
        assert_eq!(t.child_ino(42), Err(ErrnoName::EIO));
        assert_eq!(t.ino(1), Err(ErrnoName::EIO));
        t.record_name(50, 1, n("d"), InodeKind::Directory).unwrap();
        t.record_name(51, 50, n("f"), InodeKind::File).unwrap();
        t.rename(1, &n("d"), 1, n("e"));
        assert_eq!(t.path(51).unwrap().unwrap().as_str(), "e/f");
        t.record_name(51, 50, n("f"), InodeKind::File).unwrap();
        t.forget(51, 1);
        assert!(t.path(51).is_ok());
        t.forget(50, 1);
        assert_eq!(t.path(51), Err(ErrnoName::ESTALE));
        t.forget(51, 1);
        assert_eq!(t.path(51), Err(ErrnoName::ESTALE));
    }
    #[test]
    fn replaced_names_detach_and_hard_links_survive() {
        let mut t = InodeTable::new(2);
        t.record_name(3, 1, n("a"), InodeKind::File).unwrap();
        t.record_name(3, 1, n("b"), InodeKind::File).unwrap();
        t.unlink_name(1, &n("a"));
        assert_eq!(t.path(3).unwrap().unwrap().as_str(), "b");
        t.record_name(4, 1, n("c"), InodeKind::File).unwrap();
        t.rename(1, &n("c"), 1, n("b"));
        assert_eq!(t.path(3), Err(ErrnoName::ESTALE));
        assert_eq!(t.path(4).unwrap().unwrap().as_str(), "b");
        t.record_name(5, 1, n("b"), InodeKind::File).unwrap();
        assert_eq!(t.path(4), Err(ErrnoName::ESTALE));
    }
    #[test]
    fn path_bounds_and_invalid_names() {
        let mut t = InodeTable::new(2);
        let mut parent = 1;
        for id in 3..25 {
            t.record_name(id, parent, n(&"a".repeat(255)), InodeKind::Directory)
                .unwrap();
            parent = id as u64;
        }
        assert_eq!(t.path(parent), Err(ErrnoName::ENAMETOOLONG));
        assert_eq!(name(OsStr::new("..")), Err(ErrnoName::EINVAL));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(name(OsStr::from_bytes(&[255])), Err(ErrnoName::EINVAL));
        }
    }
}
