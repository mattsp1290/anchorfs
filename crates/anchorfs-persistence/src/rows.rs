//! SQL-facing records. Conversion validates database strings before exposing core vocabulary.
use anchorfs_core::{CredentialId, DeploymentId, InodeId, WorkspaceId};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SchemaMetaRow {
    pub singleton: bool,
    pub schema_version: i32,
    pub object_format_version: i32,
    pub deployment_id: String,
    pub created_at: OffsetDateTime,
}
impl TryFrom<&SchemaMetaRow> for DeploymentId {
    type Error = &'static str;
    fn try_from(row: &SchemaMetaRow) -> Result<Self, Self::Error> {
        Self::parse(row.deployment_id.clone())
    }
}
impl From<&SchemaMetaRow> for crate::meta::Versions {
    fn from(row: &SchemaMetaRow) -> Self {
        Self {
            schema_version: row.schema_version,
            object_format_version: row.object_format_version,
        }
    }
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WorkspaceRow {
    pub id: Uuid,
    pub root_inode_id: i64,
    pub state: String,
    pub authorization_epoch: i64,
    pub mutation_epoch: i64,
    pub workflow_owner_credential_id: Option<Uuid>,
    pub workflow_token_hash: Option<Vec<u8>>,
    pub workflow_expires_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
impl From<&WorkspaceRow> for WorkspaceId {
    fn from(row: &WorkspaceRow) -> Self {
        Self(row.id)
    }
}
impl From<&WorkspaceRow> for InodeId {
    fn from(row: &WorkspaceRow) -> Self {
        Self(row.root_inode_id)
    }
}
#[derive(Clone, sqlx::FromRow)]
pub struct CredentialRow {
    pub id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub kind: String,
    pub role: String,
    pub issued_by_credential_id: Option<Uuid>,
    pub uid: i64,
    pub gid: i64,
    pub secret_hash: Vec<u8>,
    pub epoch: i64,
    pub max_reservation_bytes: i64,
    pub expires_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}
impl From<&CredentialRow> for CredentialId {
    fn from(row: &CredentialRow) -> Self {
        Self(row.id)
    }
}
