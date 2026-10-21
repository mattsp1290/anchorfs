use anchorfs_client::error::ClientError;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrnoName {
    EINVAL,
    ENAMETOOLONG,
    EACCES,
    EPERM,
    ENOENT,
    EEXIST,
    EIO,
    EBUSY,
    ESTALE,
    ENOTDIR,
    EISDIR,
    ENOTEMPTY,
    EFBIG,
    EAGAIN,
    EDQUOT,
    ENOSPC,
    EOPNOTSUPP,
    ETIMEDOUT,
    ENOLCK,
    ENOSYS,
}
#[derive(Debug, Clone, Copy)]
pub enum CallClass {
    Read,
    Mutation,
}
pub fn errno_for(error: &ClientError, class: CallClass) -> ErrnoName {
    use ErrnoName::*;
    match error {
        ClientError::InvalidInput(_) => EINVAL,
        ClientError::Deadline if matches!(class, CallClass::Read) => ETIMEDOUT,
        ClientError::Api(e) => match e.code.as_str() {
            "invalid_input" | "payload_too_large" | "unsupported_media_type" => EINVAL,
            "name_too_long" => ENAMETOOLONG,
            "unauthenticated" | "fenced" | "stale_handle" => ESTALE,
            "forbidden" => EACCES,
            "not_found" => ENOENT,
            "exists" => EEXIST,
            "workspace_busy" => EBUSY,
            "not_directory" => ENOTDIR,
            "is_directory" => EISDIR,
            "directory_not_empty" => ENOTEMPTY,
            "file_too_large" => EFBIG,
            "rate_limited" => EAGAIN,
            "quota_exceeded" => EDQUOT,
            "capacity_exhausted" => ENOSPC,
            "unsupported" | "not_implemented" | "method_not_allowed" => EOPNOTSUPP,
            _ => EIO,
        },
        _ => EIO,
    }
}

#[cfg(target_os = "linux")]
impl From<ErrnoName> for fuser::Errno {
    fn from(value: ErrnoName) -> Self {
        match value {
            ErrnoName::EINVAL => Self::EINVAL,
            ErrnoName::ENAMETOOLONG => Self::ENAMETOOLONG,
            ErrnoName::EACCES => Self::EACCES,
            ErrnoName::EPERM => Self::EPERM,
            ErrnoName::ENOENT => Self::ENOENT,
            ErrnoName::EEXIST => Self::EEXIST,
            ErrnoName::EIO => Self::EIO,
            ErrnoName::EBUSY => Self::EBUSY,
            ErrnoName::ESTALE => Self::ESTALE,
            ErrnoName::ENOTDIR => Self::ENOTDIR,
            ErrnoName::EISDIR => Self::EISDIR,
            ErrnoName::ENOTEMPTY => Self::ENOTEMPTY,
            ErrnoName::EFBIG => Self::EFBIG,
            ErrnoName::EAGAIN => Self::EAGAIN,
            ErrnoName::EDQUOT => Self::EDQUOT,
            ErrnoName::ENOSPC => Self::ENOSPC,
            ErrnoName::EOPNOTSUPP => Self::EOPNOTSUPP,
            ErrnoName::ETIMEDOUT => Self::ETIMEDOUT,
            ErrnoName::ENOLCK => Self::ENOLCK,
            ErrnoName::ENOSYS => Self::ENOSYS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errno_table_matches_core_hints() {
        for code in anchorfs_core::ErrorCode::ALL {
            let e = ClientError::Api(anchorfs_client::error::ApiFailure {
                status: code.status(),
                code: code.as_str().into(),
                message: String::new(),
                trace_id: String::new(),
                operation_id: None,
                retry_after_ms: None,
            });
            let got = errno_for(&e, CallClass::Mutation);
            if *code == anchorfs_core::ErrorCode::Unauthenticated {
                assert_eq!(got, ErrnoName::ESTALE);
            } else if code.errno_hint().starts_with('E') && !code.errno_hint().contains(' ') {
                assert_eq!(format!("{got:?}"), code.errno_hint());
            } else {
                assert_eq!(got, ErrnoName::EIO);
            }
        }
        assert_eq!(
            errno_for(&ClientError::Deadline, CallClass::Read),
            ErrnoName::ETIMEDOUT
        );
        assert_eq!(
            errno_for(&ClientError::Deadline, CallClass::Mutation),
            ErrnoName::EIO
        );
    }
}
