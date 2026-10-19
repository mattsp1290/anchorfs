//! Independent in-memory tree oracle from spec 03 and approved Stage 1b decisions.
//! It derives links from directory maps rather than reproducing SQL updates.
use std::collections::BTreeMap;
pub type Outcome = Result<Option<i64>, &'static str>;
#[derive(Debug, Clone)]
pub struct Op {
    pub actor: usize,
    pub action: Action,
}
#[derive(Debug, Clone)]
pub enum Action {
    Mkdir {
        path: String,
        mode: u32,
    },
    Rmdir {
        path: String,
    },
    Symlink {
        path: String,
    },
    Link {
        existing: String,
        new_path: String,
    },
    Unlink {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Chmod {
        path: String,
        mode: u32,
    },
    Chown {
        path: String,
        uid: u32,
        gid: u32,
    },
    Open {
        path: String,
        flags: OpenFlags,
        mode: u32,
    },
    Close {
        selector: u8,
    },
}
#[derive(Debug, Clone, Copy)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
}
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: &'static str,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub children: BTreeMap<String, i64>,
    pub target: Option<String>,
    pub unlinked: bool,
    pub open_handles: u32,
}
#[derive(Debug, Clone)]
pub struct Open {
    pub node: i64,
    pub actor: usize,
}
#[derive(Debug, Clone)]
pub struct Model {
    pub nodes: BTreeMap<i64, Node>,
    pub handles: BTreeMap<i64, Open>,
    next: i64,
    next_handle: i64,
}
impl Model {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::from([(
                1,
                Node {
                    kind: "directory",
                    mode: 0o777,
                    uid: 1000,
                    gid: 1000,
                    children: BTreeMap::new(),
                    target: None,
                    unlinked: false,
                    open_handles: 0,
                },
            )]),
            handles: BTreeMap::new(),
            next: 2,
            next_handle: 1,
        }
    }
    fn identity(actor: usize) -> (u32, u32) {
        if actor == 1 {
            (2000, 1000)
        } else {
            (1000, 1000)
        }
    }
    fn access(&self, id: i64, actor: usize, bits: u32) -> Result<(), &'static str> {
        if actor == 2 {
            return Ok(());
        }
        let n = &self.nodes[&id];
        let (uid, gid) = Self::identity(actor);
        let granted = if uid == n.uid {
            n.mode / 64
        } else if gid == n.gid {
            n.mode / 8
        } else {
            n.mode
        };
        if granted & bits == bits {
            Ok(())
        } else {
            Err("forbidden")
        }
    }
    fn resolve(&self, path: &str, actor: usize) -> Result<i64, &'static str> {
        let mut id = 1;
        if !path.is_empty() {
            for name in path.split('/') {
                let n = &self.nodes[&id];
                if n.kind != "directory" {
                    return Err("not_directory");
                }
                self.access(id, actor, 1)?;
                id = *n.children.get(name).ok_or("not_found")?;
            }
        }
        Ok(id)
    }
    fn parent(&self, path: &str, actor: usize) -> Result<(i64, String), &'static str> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let id = self.resolve(parent, actor)?;
        if self.nodes[&id].kind != "directory" {
            return Err("not_directory");
        }
        self.access(id, actor, 3)?;
        Ok((id, name.to_owned()))
    }
    fn create(
        &mut self,
        path: &str,
        actor: usize,
        kind: &'static str,
        mode: u32,
        target: Option<String>,
    ) -> Result<i64, &'static str> {
        if mode > 0o777 {
            return Err("invalid_input");
        }
        let (parent, name) = self.parent(path, actor)?;
        if self.nodes[&parent].children.contains_key(&name) {
            return Err("exists");
        }
        let id = self.next;
        self.next += 1;
        let (uid, gid) = Self::identity(actor);
        self.nodes.insert(
            id,
            Node {
                kind,
                mode,
                uid,
                gid,
                children: BTreeMap::new(),
                target,
                unlinked: false,
                open_handles: 0,
            },
        );
        self.nodes
            .get_mut(&parent)
            .unwrap()
            .children
            .insert(name, id);
        Ok(id)
    }
    pub fn nlink(&self, id: i64) -> u32 {
        let n = &self.nodes[&id];
        if n.kind == "directory" {
            if n.unlinked {
                0
            } else {
                2 + n
                    .children
                    .values()
                    .filter(|id| self.nodes[id].kind == "directory")
                    .count() as u32
            }
        } else {
            self.nodes
                .values()
                .map(|n| n.children.values().filter(|v| **v == id).count() as u32)
                .sum()
        }
    }
    fn mark_unlinked(&mut self, id: i64) {
        if self.nlink(id) == 0 || self.nodes[&id].kind == "directory" {
            self.nodes.get_mut(&id).unwrap().unlinked = true;
        }
    }
    fn descendant(&self, ancestor: i64, candidate: i64) -> bool {
        ancestor == candidate
            || self.nodes[&ancestor].children.values().any(|child| {
                self.nodes[child].kind == "directory" && self.descendant(*child, candidate)
            })
    }
    pub fn selected_handle(&self, selector: u8) -> Option<i64> {
        if self.handles.is_empty() {
            None
        } else {
            self.handles
                .keys()
                .nth(usize::from(selector) % self.handles.len())
                .copied()
        }
    }
    pub fn apply(&mut self, op: &Op) -> Outcome {
        let actor = op.actor;
        match &op.action {
            Action::Mkdir { path, mode } => {
                Ok(Some(self.create(path, actor, "directory", *mode, None)?))
            }
            Action::Rmdir { path } => self.remove(actor, path, true),
            Action::Symlink { path } => Ok(Some(self.create(
                path,
                actor,
                "symlink",
                0o777,
                Some("/opaque".into()),
            )?)),
            Action::Link { existing, new_path } => {
                let id = self.resolve(existing, actor)?;
                if self.nodes[&id].kind == "directory" {
                    return Err("is_directory");
                }
                let (p, name) = self.parent(new_path, actor)?;
                if self.nodes[&p].children.contains_key(&name) {
                    return Err("exists");
                }
                self.nodes.get_mut(&p).unwrap().children.insert(name, id);
                Ok(Some(id))
            }
            Action::Unlink { path } => self.remove(actor, path, false),
            Action::Rename { from, to } => self.rename(actor, from, to),
            Action::Chmod { path, mode } => {
                if *mode > 0o777 {
                    return Err("invalid_input");
                }
                let id = self.resolve(path, actor)?;
                let n = &self.nodes[&id];
                if n.kind == "symlink" {
                    return Err("invalid_input");
                }
                if actor != 2 && n.uid != Self::identity(actor).0 {
                    return Err("forbidden");
                }
                self.nodes.get_mut(&id).unwrap().mode = *mode;
                Ok(Some(id))
            }
            Action::Chown { path, uid, gid } => {
                let id = self.resolve(path, actor)?;
                if actor != 2 {
                    return Err("forbidden");
                }
                let n = self.nodes.get_mut(&id).unwrap();
                n.uid = *uid;
                n.gid = *gid;
                Ok(Some(id))
            }
            Action::Open { path, flags, mode } => self.open(actor, path, *flags, *mode),
            Action::Close { selector } => {
                let selected = self.selected_handle(*selector).ok_or("not_found")?;
                let h = &self.handles[&selected];
                if h.actor != actor {
                    return Err("not_found");
                }
                let id = h.node;
                self.handles.remove(&selected);
                self.nodes.get_mut(&id).unwrap().open_handles -= 1;
                Ok(Some(id))
            }
        }
    }
    fn remove(&mut self, actor: usize, path: &str, directory: bool) -> Outcome {
        let (p, name) = self.parent(path, actor)?;
        let id = *self.nodes[&p].children.get(&name).ok_or("not_found")?;
        let n = &self.nodes[&id];
        if directory && n.kind != "directory" {
            return Err("not_directory");
        }
        if !directory && n.kind == "directory" {
            return Err("is_directory");
        }
        if directory && !n.children.is_empty() {
            return Err("directory_not_empty");
        }
        self.nodes.get_mut(&p).unwrap().children.remove(&name);
        self.mark_unlinked(id);
        Ok(Some(id))
    }
    fn rename(&mut self, actor: usize, from: &str, to: &str) -> Outcome {
        let (pf, nf) = self.parent(from, actor)?;
        let (pt, nt) = self.parent(to, actor)?;
        let src = *self.nodes[&pf].children.get(&nf).ok_or("not_found")?;
        let dst = self.nodes[&pt].children.get(&nt).copied();
        if from == to || dst == Some(src) {
            return Ok(Some(src));
        }
        let is_dir = self.nodes[&src].kind == "directory";
        if is_dir && pf != pt {
            self.access(src, actor, 2)?;
            if self.descendant(src, pt) {
                return Err("invalid_input");
            }
        }
        if let Some(dst) = dst {
            let d = &self.nodes[&dst];
            if is_dir && d.kind != "directory" {
                return Err("not_directory");
            }
            if !is_dir && d.kind == "directory" {
                return Err("is_directory");
            }
            if d.kind == "directory" && !d.children.is_empty() {
                return Err("directory_not_empty");
            }
            self.nodes.get_mut(&pt).unwrap().children.remove(&nt);
            self.mark_unlinked(dst);
        }
        self.nodes.get_mut(&pf).unwrap().children.remove(&nf);
        self.nodes.get_mut(&pt).unwrap().children.insert(nt, src);
        Ok(Some(src))
    }
    fn open(&mut self, actor: usize, path: &str, flags: OpenFlags, mode: u32) -> Outcome {
        let OpenFlags {
            read,
            write,
            create,
            exclusive: excl,
            truncate,
        } = flags;
        if (!read && !write)
            || (excl && !create)
            || (truncate && !write)
            || (create && mode > 0o777)
        {
            return Err("invalid_input");
        }
        let existing = if create {
            let (p, name) = self.parent(path, actor)?;
            self.nodes[&p].children.get(&name).copied()
        } else {
            Some(self.resolve(path, actor)?)
        };
        let id = if let Some(id) = existing {
            if excl {
                return Err("exists");
            }
            let n = &self.nodes[&id];
            if n.kind == "directory" {
                return Err("is_directory");
            }
            if n.kind == "symlink" {
                return Err("invalid_input");
            }
            self.access(id, actor, u32::from(read) * 4 + u32::from(write) * 2)?;
            id
        } else {
            self.create(path, actor, "file", mode, None)?
        };
        let h = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(h, Open { node: id, actor });
        self.nodes.get_mut(&id).unwrap().open_handles += 1;
        Ok(Some(id))
    }
    pub fn paths(&self) -> Vec<(String, i64)> {
        fn walk(m: &Model, id: i64, path: String, out: &mut Vec<(String, i64)>) {
            out.push((path.clone(), id));
            for (name, child) in &m.nodes[&id].children {
                walk(
                    m,
                    *child,
                    if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}/{name}")
                    },
                    out,
                );
            }
        }
        let mut out = vec![];
        walk(self, 1, String::new(), &mut out);
        out
    }
}
