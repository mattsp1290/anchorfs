use anchorfs_core::{
    digest::Sha256Digest,
    ids::{DeploymentId, ObjectId, WorkspaceId},
};
use std::fmt;
/// Opaque backend identity: user filesystem paths cannot enter its constructor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectKey(String);
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("expected three key segments")]
    Segments,
    #[error("foreign deployment")]
    ForeignDeployment,
    #[error("invalid canonical UUID")]
    Identifier,
}
impl ObjectKey {
    pub fn deployment_prefix(deployment: &DeploymentId) -> String {
        format!("{deployment}/")
    }
    pub fn parse(raw: &str, deployment: &DeploymentId) -> Result<Self, KeyError> {
        let parts: Vec<_> = raw.split('/').collect();
        if parts.len() != 3 {
            return Err(KeyError::Segments);
        }
        if parts[0] != deployment.as_str() {
            return Err(KeyError::ForeignDeployment);
        }
        // Canonical UUID text only: no normalization or alternate encodings.
        for part in &parts[1..] {
            if part.len() != 36
                || !part.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                    }
                })
            {
                return Err(KeyError::Identifier);
            }
        }
        Ok(Self(raw.into()))
    }

    pub fn new(deployment: &DeploymentId, workspace: WorkspaceId, object: ObjectId) -> Self {
        Self(format!("{deployment}/{workspace}/{object}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for ObjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub deployment_id: DeploymentId,
    pub workspace_id: WorkspaceId,
    pub object_id: ObjectId,
    pub sha256: Sha256Digest,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_namespace_and_cache_integrity() {
        let deployment = DeploymentId::parse("local-01").unwrap();
        let workspace = WorkspaceId::new();
        let object = ObjectId::new();
        let key = ObjectKey::new(&deployment, workspace, object);
        let parts: Vec<_> = key.as_str().split('/').collect();
        assert_eq!(parts.len(), 3);
        assert!((1..=64).contains(&parts[0].len()));
        assert!(
            parts[0]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        );
        for part in &parts[1..] {
            assert_eq!(part.len(), 36);
            assert!(
                part.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
            );
        }
        assert_eq!(
            key.to_string(),
            format!("{deployment}/{workspace}/{object}")
        );
        let cache = CacheKey {
            deployment_id: deployment,
            workspace_id: workspace,
            object_id: object,
            sha256: Sha256Digest([0; 32]),
        };
        let mut different = cache.clone();
        different.sha256 = Sha256Digest([1; 32]);
        assert_ne!(cache, different);
        different = cache.clone();
        different.deployment_id = DeploymentId::parse("other").unwrap();
        assert_ne!(cache, different);
    }
    #[test]
    fn key_parse_accepts_own_deployment_rejects_foreign_and_malformed() {
        let d = DeploymentId::parse("test").unwrap();
        let k = ObjectKey::new(&d, WorkspaceId::new(), ObjectId::new());
        assert_eq!(ObjectKey::parse(k.as_str(), &d), Ok(k.clone()));
        assert_eq!(ObjectKey::deployment_prefix(&d), "test/");
        assert_eq!(ObjectKey::parse("test/x", &d), Err(KeyError::Segments));
        assert_eq!(
            ObjectKey::parse("foreign/x/y", &d),
            Err(KeyError::ForeignDeployment)
        );
        assert_eq!(
            ObjectKey::parse("test/not-uuid/x", &d),
            Err(KeyError::Identifier)
        );
        assert_eq!(
            ObjectKey::parse(&k.as_str().to_uppercase().replacen("TEST/", "test/", 1), &d),
            Err(KeyError::Identifier)
        );
    }
}
