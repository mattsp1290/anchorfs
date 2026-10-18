//! Focused control-plane acceptance modules; all scenarios retain this test target.
#[path = "control_plane/authority.rs"]
mod authority;
mod common;
#[path = "control_plane/contracts.rs"]
mod contracts;
#[path = "control_plane/credentials.rs"]
mod credentials;
#[path = "control_plane/failure_recovery.rs"]
mod failure_recovery;
#[path = "control_plane/namespace_mutations.rs"]
mod namespace_mutations;
#[path = "control_plane/namespace_reads.rs"]
mod namespace_reads;
#[path = "control_plane/publication.rs"]
mod publication;
#[path = "control_plane/receipts.rs"]
mod receipts;
#[path = "control_plane/sessions.rs"]
mod sessions;
#[path = "control_plane/support.rs"]
mod support;
