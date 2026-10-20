//! Synthetic byte-state tests use disposable PostgreSQL only.
#[path = "bytes_tx/accounting.rs"]
mod accounting;
mod common;
#[path = "bytes_tx/manifests.rs"]
mod manifests;
#[path = "bytes_tx/support.rs"]
mod support;
