//! Synthetic byte-state tests use disposable PostgreSQL only.
#[path = "bytes_tx/accounting.rs"]
mod accounting;
mod common;
#[path = "bytes_tx/handles.rs"]
mod handles;
#[path = "bytes_tx/invariants.rs"]
mod invariants;
#[path = "bytes_tx/manifests.rs"]
mod manifests;
#[path = "bytes_tx/receipts.rs"]
mod receipts;
#[path = "bytes_tx/support.rs"]
mod support;
#[path = "bytes_tx/workflows.rs"]
mod workflows;
