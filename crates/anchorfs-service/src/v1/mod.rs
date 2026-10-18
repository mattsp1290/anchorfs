pub mod bytes;
pub mod credentials;
pub mod handles;
pub mod namespace;
pub mod operations;
pub mod pins;
pub mod sessions;
pub mod workflows;
pub mod workspaces;
fn validate_path(path: &str, trace: &str) -> Result<(), crate::error::ApiError> {
    anchorfs_core::WorkspacePath::parse(path)
        .map(|_| ())
        .map_err(|err| {
            crate::error::ApiError::new(
                if err == anchorfs_core::PathError::TooLong {
                    anchorfs_core::ErrorCode::NameTooLong
                } else {
                    anchorfs_core::ErrorCode::InvalidInput
                },
                trace,
            )
        })
}
/// Independently selected route identities; spec 03 Native API plus spec 02 read pins and spec 07 lifecycle.
pub const REQUIRED_OPERATIONS: &[&str] = &[
    "createWorkspace",
    "getWorkspace",
    "createCredential",
    "rotateCredential",
    "revokeCredential",
    "createSession",
    "renewSession",
    "rebindSession",
    "revokeSession",
    "statPath",
    "listDirectory",
    "createDirectory",
    "removeDirectory",
    "openHandle",
    "renewHandle",
    "closeHandle",
    "readRange",
    "writeAt",
    "append",
    "truncate",
    "fsync",
    "createReadPin",
    "releaseReadPin",
    "replaceFile",
    "unlink",
    "createLink",
    "createSymlink",
    "readSymlink",
    "rename",
    "setAttributes",
    "getOperation",
    "findOperation",
    "getAdminOperation",
    "findAdminOperation",
    "acquireWorkflowLease",
    "renewWorkflowLease",
    "releaseWorkflowLease",
    "takeoverWorkflowLease",
];
pub mod views;

use crate::{error::ApiError, extract::SessionBinding};
use anchorfs_core::{CredentialProof, SessionId, WorkspaceId};
use anchorfs_persistence::{
    authority::{
        self, ActorCredential, ActorSession, AuthorityError, LeaseMode, LockMode, WorkflowProof,
        WorkspaceLock,
    },
    namespace::Caller,
};
pub struct Gate {
    pub role_mutation: bool,
    pub lease: LeaseMode,
}
pub struct Prologue {
    pub actor: ActorCredential,
    pub session: ActorSession,
    pub workspace: WorkspaceLock,
    pub caller: Caller,
}
pub async fn prologue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    proof: &CredentialProof,
    workspace: WorkspaceId,
    binding: &SessionBinding,
    workflow: Option<&WorkflowProof>,
    gate: Gate,
) -> Result<Prologue, ApiError> {
    let lock = authority::lock_workspace(tx, workspace).await?;
    let actor = authority::load_actor(tx, proof, LockMode::Share).await?;
    if !authority::same_workspace(&actor, workspace) {
        return Err(AuthorityError::NotFound.into());
    }
    let lock = lock.ok_or(AuthorityError::NotFound)?;
    let session = authority::load_actor_session(
        tx,
        &actor,
        workspace,
        SessionId(binding.session_id),
        binding.session_epoch,
        LockMode::Share,
    )
    .await?;
    if gate.role_mutation && !authority::role_permits_mutation(&actor) {
        return Err(AuthorityError::Forbidden.into());
    }
    authority::check_workflow(tx, &lock, &actor, &session, workflow, gate.lease).await?;
    let caller = Caller {
        identity: authority::identity(&actor),
        admin: authority::is_workspace_admin(&actor),
    };
    Ok(Prologue {
        actor,
        session,
        workspace: lock,
        caller,
    })
}
