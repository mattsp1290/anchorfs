use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InodeKind {
    File,
    Directory,
    Symlink,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode(pub u32);
impl Mode {
    pub fn is_setuid(self) -> bool {
        self.0 & 0o4000 != 0
    }
    pub fn is_setgid(self) -> bool {
        self.0 & 0o2000 != 0
    }
    pub fn is_sticky(self) -> bool {
        self.0 & 0o1000 != 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privileged_bits() {
        assert!(Mode(0o4755).is_setuid());
        assert!(Mode(0o2755).is_setgid());
        assert!(Mode(0o1777).is_sticky());
        assert!(!Mode(0o755).is_setuid());
    }
}

/// The single uid/gid authorized by a credential; no supplementary groups.
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Access {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
}
pub fn permits(mode: Mode, owner_uid: u32, owner_gid: u32, who: Identity, want: Access) -> bool {
    let shift = if who.uid == owner_uid {
        6
    } else if who.gid == owner_gid {
        3
    } else {
        0
    };
    let requested =
        (u32::from(want.read) << 2) | (u32::from(want.write) << 1) | u32::from(want.exec);
    (mode.0 >> shift) & requested == requested
}
#[cfg(test)]
mod permission_tests {
    use super::*;
    #[test]
    fn permission_class_and_bit_matrix() {
        for (who, shift) in [
            (Identity { uid: 1, gid: 2 }, 6),
            (Identity { uid: 3, gid: 2 }, 3),
            (Identity { uid: 3, gid: 4 }, 0),
        ] {
            for bits in 0..8 {
                for requested in 0..8 {
                    let want = Access {
                        read: requested & 4 != 0,
                        write: requested & 2 != 0,
                        exec: requested & 1 != 0,
                    };
                    assert_eq!(
                        permits(Mode(bits << shift), 1, 2, who, want),
                        bits & requested == requested
                    );
                }
            }
        }
        assert!(!permits(
            Mode(0o077),
            1,
            2,
            Identity { uid: 1, gid: 2 },
            Access {
                read: true,
                ..Access::default()
            }
        ));
    }
}
