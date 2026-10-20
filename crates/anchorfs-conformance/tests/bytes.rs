//! Durable-byte acceptance uses disposable PostgreSQL and pinned MinIO only.
mod common;
#[path = "bytes/fixture.rs"]
mod fixture;
#[path = "bytes/support.rs"]
mod support;
#[path = "bytes/writes.rs"]
mod writes;
