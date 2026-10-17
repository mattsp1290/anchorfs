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
