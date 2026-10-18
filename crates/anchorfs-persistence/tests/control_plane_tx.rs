//! Focused control-plane acceptance modules; all scenarios retain this test target.
#[path = "control_plane_tx/admission.rs"]
mod admission;
mod common;
#[path = "control_plane_tx/contention.rs"]
mod contention;
#[path = "control_plane_tx/credentials.rs"]
mod credentials;
#[path = "control_plane_tx/publication.rs"]
mod publication;
#[path = "control_plane_tx/sessions.rs"]
mod sessions;
#[path = "control_plane_tx/support.rs"]
mod support;
#[path = "control_plane_tx/workspaces.rs"]
mod workspaces;
