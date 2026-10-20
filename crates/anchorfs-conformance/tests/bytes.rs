//! Durable-byte acceptance uses disposable PostgreSQL and pinned MinIO only.
#[path = "bytes/access.rs"]
mod access;
mod common;
#[path = "bytes/concurrency.rs"]
mod concurrency;
#[path = "bytes/fixture.rs"]
mod fixture;
#[path = "bytes/quotas.rs"]
mod quotas;
#[path = "bytes/reads.rs"]
mod reads;
#[path = "bytes/recovery.rs"]
mod recovery;
#[path = "bytes/replace.rs"]
mod replace;
#[path = "bytes/support.rs"]
mod support;
#[path = "bytes/writes.rs"]
mod writes;
