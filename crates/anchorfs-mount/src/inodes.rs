use crate::errno::ErrnoName;
use anchorfs_core::{Name, PathError, WorkspacePath};
use std::{collections::BTreeMap, ffi::OsStr};

#[derive(Debug)]
pub struct Node {
    pub names: Vec<(u64, Name)>,
    pub nlookup: u64,
    pub detached: bool,
}
pub struct InodeTable {
    root: i64,
    nodes: BTreeMap<u64, Node>,
    names: BTreeMap<(u64, String), u64>,
}
impl InodeTable {
    pub fn new(root: i64) -> Self {
        Self {
            root,
            names: BTreeMap::new(),
            nodes: BTreeMap::from([(
                1,
                Node {
                    names: vec![],
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
    ) -> Result<u64, ErrnoName> {
        let ino = self.child_ino(server_id)?;
        // A fresh lookup can observe a different inode at a known name.
        self.unlink_name(parent, &name);
        let node = self.nodes.entry(ino).or_insert(Node {
            names: vec![],
            nlookup: 0,
            detached: false,
        });
        // A lookup proves this alias is current. Prefer it over an older
        // alias which another mount may have removed without notifying us.
        node.names.retain(|(p, n)| *p != parent || *n != name);
        node.names.insert(0, (parent, name.clone()));
        node.nlookup = node.nlookup.saturating_add(1);
        node.detached = false;
        self.names.insert((parent, name.as_str().to_owned()), ino);
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
        if let Some(ino) = self.names.remove(&(parent, name.as_str().to_owned()))
            && let Some(node) = self.nodes.get_mut(&ino)
        {
            node.names.retain(|(p, n)| *p != parent || n != name);
            if ino != 1 && node.names.is_empty() {
                node.detached = true;
            }
        }
    }
    pub fn child(&self, parent: u64, name: &Name) -> Option<u64> {
        self.names.get(&(parent, name.as_str().to_owned())).copied()
    }
    pub fn rename(&mut self, parent: u64, name: &Name, newparent: u64, newname: Name) {
        let source = self.child(parent, name);
        let target = self.child(newparent, &newname);
        if source.is_some() && source == target {
            return;
        } // POSIX same-inode rename is a no-op.
        self.unlink_name(newparent, &newname);
        if let Some(id) = source
            && let Some(node) = self.nodes.get_mut(&id)
        {
            self.names.remove(&(parent, name.as_str().to_owned()));
            self.names
                .insert((newparent, newname.as_str().to_owned()), id);
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
                let node = self.nodes.remove(&ino).expect("node exists");
                for alias in node.names {
                    self.names.remove(&(alias.0, alias.1.as_str().to_owned()));
                }
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
        t.record_name(50, 1, n("d")).unwrap();
        t.record_name(51, 50, n("f")).unwrap();
        t.rename(1, &n("d"), 1, n("e"));
        assert_eq!(t.path(51).unwrap().unwrap().as_str(), "e/f");
        t.record_name(51, 50, n("f")).unwrap();
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
        t.record_name(3, 1, n("a")).unwrap();
        t.record_name(3, 1, n("b")).unwrap();
        t.unlink_name(1, &n("a"));
        assert_eq!(t.path(3).unwrap().unwrap().as_str(), "b");
        t.record_name(4, 1, n("c")).unwrap();
        t.rename(1, &n("c"), 1, n("b"));
        assert_eq!(t.path(3), Err(ErrnoName::ESTALE));
        assert_eq!(t.path(4).unwrap().unwrap().as_str(), "b");
        t.record_name(5, 1, n("b")).unwrap();
        assert_eq!(t.path(4), Err(ErrnoName::ESTALE));
    }
    #[test]
    fn fresh_lookup_prefers_the_verified_hard_link_alias() {
        let mut table = InodeTable::new(2);
        table.record_name(3, 1, n("old")).unwrap();
        table.record_name(3, 1, n("live")).unwrap();
        table.record_name(3, 1, n("old")).unwrap();
        assert_eq!(table.path(3).unwrap().unwrap().as_str(), "old");
        table.detach(3);
        table.record_name(3, 1, n("live")).unwrap();
        assert_eq!(table.path(3).unwrap().unwrap().as_str(), "live");
    }
    #[test]
    fn reverse_index_tracks_replacement_rename_unlink_and_forget() {
        let mut table = InodeTable::new(2);
        let mut seed = 7_u64;
        for _ in 0..2000 {
            // Deterministic synthetic operation sequences; no fixture source.
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = 3 + ((seed >> 32) % 16);
            let a = n(&format!("n{}", (seed >> 16) % 24));
            let b = n(&format!("n{}", (seed >> 8) % 24));
            match seed % 5 {
                0 | 1 => {
                    table.record_name(id as i64, 1, a).unwrap();
                }
                2 => table.rename(1, &a, 1, b),
                3 => table.unlink_name(1, &a),
                _ => table.forget(id, 1),
            }
            let rebuilt: BTreeMap<_, _> = table
                .nodes
                .iter()
                .flat_map(|(&ino, node)| {
                    node.names
                        .iter()
                        .map(move |alias| ((alias.0, alias.1.as_str().to_owned()), ino))
                })
                .collect();
            assert_eq!(table.names, rebuilt);
            for (&ino, node) in &table.nodes {
                if ino != 1 {
                    assert_eq!(node.detached, node.names.is_empty());
                }
            }
        }
    }
    #[test]
    fn path_bounds_and_invalid_names() {
        let mut t = InodeTable::new(2);
        let mut parent = 1;
        for id in 3..25 {
            t.record_name(id, parent, n(&"a".repeat(255))).unwrap();
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
