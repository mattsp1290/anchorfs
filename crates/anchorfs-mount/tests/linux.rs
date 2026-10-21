#![cfg(target_os = "linux")]
// Raw flock/fcntl/mmap/setxattr/mknod/renameat2 and effective-id syscalls
// are confined to this disposable, Linux-only acceptance test binary.
#![allow(unsafe_code)]
#[path = "linux/cli.rs"]
mod cli;
#[path = "linux/fencing.rs"]
mod fencing;
#[path = "linux/fixture.rs"]
mod fixture;
#[path = "linux/git_smoke.rs"]
mod git_smoke;
#[path = "linux/handles.rs"]
mod handles;
#[path = "linux/lifecycle.rs"]
mod lifecycle;
#[path = "../../anchorfs-conformance/tests/common/minio.rs"]
mod minio;
#[path = "linux/mountpoint.rs"]
mod mountpoint;
#[allow(dead_code)] // Reused fault-injection methods are exercised by client conformance.
#[path = "../../anchorfs-conformance/tests/common/api_proxy.rs"]
mod relay;
#[path = "linux/unsupported.rs"]
mod unsupported;
#[path = "linux/visibility.rs"]
mod visibility;
