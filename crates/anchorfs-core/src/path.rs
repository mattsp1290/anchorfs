use std::fmt;
use thiserror::Error;
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("invalid UTF-8")]
    InvalidUtf8,
    #[error("empty path or segment")]
    Empty,
    #[error("absolute path")]
    Absolute,
    #[error("dot segment")]
    Dot,
    #[error("NUL in path")]
    Nul,
    #[error("name or path too long")]
    TooLong,
    #[error("slash in name")]
    Slash,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(String);
impl Name {
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value.is_empty() {
            return Err(PathError::Empty);
        }
        if value.len() > 255 {
            return Err(PathError::TooLong);
        }
        if value.contains('\0') {
            return Err(PathError::Nul);
        }
        if value == "." || value == ".." {
            return Err(PathError::Dot);
        }
        if value.contains('/') {
            return Err(PathError::Slash);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePath(String);
impl WorkspacePath {
    pub fn parse(value: impl AsRef<[u8]>) -> Result<Self, PathError> {
        Self::parse_with_limit(value, 4096)
    }
    pub fn parse_with_limit(value: impl AsRef<[u8]>, max_bytes: usize) -> Result<Self, PathError> {
        let value = std::str::from_utf8(value.as_ref()).map_err(|_| PathError::InvalidUtf8)?;
        if value.starts_with('/') {
            return Err(PathError::Absolute);
        }
        if value.len() > max_bytes {
            return Err(PathError::TooLong);
        }
        for name in value.split('/') {
            Name::parse(name)?;
        }
        Ok(Self(value.to_owned()))
    }
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
    pub fn split_parent(&self) -> (Option<WorkspacePath>, Name) {
        match self.0.rsplit_once('/') {
            Some((parent, name)) => (Some(Self(parent.to_owned())), Name(name.to_owned())),
            None => (None, Name(self.0.clone())),
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for WorkspacePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_rules_and_roundtrip() {
        let valid = WorkspacePath::parse("a/b/c").unwrap();
        assert_eq!(WorkspacePath::parse(valid.to_string()).unwrap(), valid);
        for bad in [
            "/a",
            "a/../b",
            "a//b",
            "a/./b",
            "",
            "a\0b",
            &"a".repeat(256),
        ] {
            assert!(WorkspacePath::parse(bad).is_err(), "{bad:?}");
        }
        assert!(WorkspacePath::parse([255]).is_err());
        let long = vec!["a"; 2049].join("/");
        assert_eq!(long.len(), 4097);
        assert_eq!(WorkspacePath::parse(long), Err(PathError::TooLong));
        assert!(Name::parse(&"é".repeat(128)).is_err());
    }
}
#[cfg(test)]
mod component_tests {
    use super::*;
    #[test]
    fn split_parent_and_components() {
        let path = WorkspacePath::parse("a/b/c").unwrap();
        assert_eq!(path.components().collect::<Vec<_>>(), vec!["a", "b", "c"]);
        let (parent, name) = path.split_parent();
        assert_eq!(parent.unwrap().as_str(), "a/b");
        assert_eq!(name.as_str(), "c");
        let (parent, name) = WorkspacePath::parse("a").unwrap().split_parent();
        assert!(parent.is_none());
        assert_eq!(name.as_str(), "a");
    }
}
