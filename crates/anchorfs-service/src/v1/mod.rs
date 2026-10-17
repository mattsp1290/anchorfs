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
