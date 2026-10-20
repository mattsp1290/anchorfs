//! Durable-byte acceptance uses disposable PostgreSQL and pinned MinIO only.
#[path = "bytes/access.rs"]
mod access;
#[path = "bytes/backend_faults.rs"]
mod backend_faults;
mod common;
#[path = "bytes/concurrency.rs"]
mod concurrency;
#[path = "bytes/fencing.rs"]
mod fencing;
#[path = "bytes/fixture.rs"]
mod fixture;
#[path = "bytes/pool.rs"]
mod pool;
#[path = "bytes/quotas.rs"]
mod quotas;
#[path = "bytes/reads.rs"]
mod reads;
#[path = "bytes/recovery.rs"]
mod recovery;
#[path = "bytes/replace.rs"]
mod replace;
#[path = "bytes/stress.rs"]
mod stress;
#[path = "bytes/support.rs"]
mod support;
#[path = "bytes/truncate.rs"]
mod truncate;
#[path = "bytes/writes.rs"]
mod writes;
