//! Shared HTTP test support; fixture implementation lives in common.
pub(super) use crate::common;
pub(super) use crate::common::fixture::*;
pub(super) use anchorfs_core::CredentialId;
pub(super) use axum::{
    body::Body,
    http::{Request, StatusCode},
};
