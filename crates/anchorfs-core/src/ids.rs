use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Random opaque workspace identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(pub Uuid);
impl WorkspaceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque manifest identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManifestId(pub Uuid);
impl ManifestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ManifestId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for ManifestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque object identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub Uuid);
impl ObjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ObjectId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque handle identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HandleId(pub Uuid);
impl HandleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for HandleId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for HandleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque operation identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(pub Uuid);
impl OperationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque credential identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialId(pub Uuid);
impl CredentialId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for CredentialId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque session identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);
impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque readpin identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReadPinId(pub Uuid);
impl ReadPinId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ReadPinId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for ReadPinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
/// Random opaque reservation identity (UUIDv4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReservationId(pub Uuid);
impl ReservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for ReservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InodeId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct DeploymentId(String);
impl DeploymentId {
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("deployment id must be 1–64 lowercase ASCII letters, digits or hyphens");
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for DeploymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl<'de> Deserialize<'de> for DeploymentId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deployment_id_bounds() {
        for bad in ["", "UPPER", "a/b", "a_b", &"a".repeat(65)] {
            assert!(DeploymentId::parse(bad).is_err());
        }
        assert!(DeploymentId::parse("local-01").is_ok());
        assert_eq!(
            WorkspaceId::new().0.get_version(),
            Some(uuid::Version::Random)
        );
    }
}
