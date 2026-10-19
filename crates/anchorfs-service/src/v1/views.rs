//! Persistence-to-wire conversions construct the same DTOs that generate OpenAPI.
use crate::{control::timestamp, dto::*, error::ApiError};
use anchorfs_core::{ErrorCode, OperationId};
use anchorfs_persistence::{
    credentials::Issued, receipts::ReceiptView, sessions::SessionView, workspaces::WorkspaceView,
};
fn invalid_record() -> ApiError {
    ApiError::new(ErrorCode::DependencyUnavailable, "")
}
pub fn workspace(view: WorkspaceView) -> Result<WorkspaceInfo, ApiError> {
    let kind = match view.kind.as_str() {
        "file" => InodeKindDto::File,
        "directory" => InodeKindDto::Directory,
        "symlink" => InodeKindDto::Symlink,
        _ => return Err(invalid_record()),
    };
    Ok(WorkspaceInfo {
        workspace_id: view.id,
        authorization_epoch: view.authorization_epoch,
        mutation_epoch: view.mutation_epoch,
        root: InodeAttributes {
            inode_id: view.inode_id,
            generation: view.generation,
            kind,
            size: view.size.try_into().map_err(|_| invalid_record())?,
            mode: view.mode.try_into().map_err(|_| invalid_record())?,
            uid: view.uid.try_into().map_err(|_| invalid_record())?,
            gid: view.gid.try_into().map_err(|_| invalid_record())?,
            nlink: view.nlink.try_into().map_err(|_| invalid_record())?,
            atime: timestamp(view.atime),
            mtime: timestamp(view.mtime),
            ctime: timestamp(view.ctime),
        },
    })
}
pub fn credential(operation: OperationId, issued: Issued) -> IssuedCredential {
    IssuedCredential {
        operation_id: operation.0,
        credential_id: issued.id.0,
        secret: Some(issued.secret.expose().clone()),
        credential_epoch: issued.epoch,
        expires_at: issued.expires_at.map(timestamp),
    }
}
pub fn session(operation: OperationId, session: SessionView) -> SessionGrant {
    SessionGrant {
        operation_id: operation.0,
        session_id: session.id.0,
        session_epoch: session.session_epoch,
        credential_epoch: session.credential_epoch,
        bound_mutation_epoch: session.bound_mutation_epoch,
        expires_at: timestamp(session.expires_at),
    }
}
pub fn mutation(operation: OperationId) -> MutationResult {
    MutationResult {
        operation_id: operation.0,
        inode_id: None,
        generation: None,
    }
}
pub fn operation(view: ReceiptView) -> Result<OperationStatus, ApiError> {
    let state = match view.state.as_str() {
        "pending" => OperationStateDto::Pending,
        "committed" => OperationStateDto::Committed,
        "failed" => OperationStateDto::Failed,
        "indeterminate" => OperationStateDto::Indeterminate,
        _ => return Err(invalid_record()),
    };
    Ok(OperationStatus {
        operation_id: view.operation_id.0,
        state,
        result: view.result,
    })
}
pub fn attributes(
    row: &anchorfs_persistence::namespace::InodeRow,
) -> Result<InodeAttributes, ApiError> {
    Ok(InodeAttributes {
        inode_id: row.id,
        generation: row.generation,
        kind: match row.kind.as_str() {
            "file" => InodeKindDto::File,
            "directory" => InodeKindDto::Directory,
            "symlink" => InodeKindDto::Symlink,
            _ => return Err(invalid_record()),
        },
        size: row.size.try_into().map_err(|_| invalid_record())?,
        mode: row.mode.try_into().map_err(|_| invalid_record())?,
        uid: row.uid.try_into().map_err(|_| invalid_record())?,
        gid: row.gid.try_into().map_err(|_| invalid_record())?,
        nlink: row.nlink.try_into().map_err(|_| invalid_record())?,
        atime: timestamp(row.atime),
        mtime: timestamp(row.mtime),
        ctime: timestamp(row.ctime),
    })
}
pub fn handle(
    operation: OperationId,
    handle: &anchorfs_persistence::handles::HandleRow,
    inode: &anchorfs_persistence::namespace::InodeRow,
) -> Result<HandleGrant, ApiError> {
    Ok(HandleGrant {
        operation_id: operation.0,
        handle_id: handle.id,
        attributes: attributes(inode)?,
        access_read: handle.access_read,
        access_write: handle.access_write,
        expires_at: timestamp(handle.expires_at),
    })
}
