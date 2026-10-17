# Requirements and scope

## Product outcome

Provide a persistent workspace mounted at a stable Linux path by multiple agent pods. An agent can create/edit files, use Git, run builds, and recover its workspace after a pod restart. Rust and Go applications consume the same storage through a mount or language-neutral API. Contents reside in S3-compatible storage; transactional metadata lives in PostgreSQL.

This document describes this project's requirements, not another product's implementation. The scope is storage; execution infrastructure is a later integration.

## Required release behavior

| ID | Requirement |
|---|---|
| R1 | Rust service and Linux mount client; build amd64 and arm64 images. |
| R2 | Exercise the same backend contract on local MinIO and DigitalOcean Spaces; AWS S3 is the subsequent provider target. |
| R3 | Multiple pods mount one workspace and edit different files concurrently. Same-file mutation operations serialize through the metadata authority. |
| R4 | Acknowledged writes survive client/server process loss and cache loss, subject to PostgreSQL and object-store durability. Never acknowledge durable success based only on local cache. |
| R5 | Atomic namespace operations inside a workspace, stable inode identity, random I/O, truncation, append, executable bits, symlinks, hard links, and unlink-open behavior in the scoped Linux profile. |
| R6 | Coherent reads in the default profile; no correctness dependency on lossy cache invalidation notifications. |
| R7 | Workspace-scoped authorization, quotas, retry-safe mutations, explicit failure reporting, and restore verification. |
| R8 | A documented integration boundary for Crabber/eino-agent; no replacement of their transactional session stores. |

## Initial exclusions

No transparent sync of pre-existing bucket keys, external modifications of managed objects, arbitrary bucket import, Windows/macOS mounts, complete POSIX certification, database-on-filesystem guarantee, writable mmap, distributed flock/fcntl locks, file ACLs, device nodes, sockets/FIFOs, or full renameat2 flag coverage. Unsupported calls must return a defined error, never pretend success. Do not describe the first release as fully POSIX-compatible.

No asynchronous durable cache tier, consensus protocol invented for this project, cross-region active-active filesystem, automatic conflict merging, filesystem snapshots/branches, S3 gateway, or command execution service in the initial release. Later stages are specified separately and must not weaken the initial guarantees.

## Concurrency meaning

Multiple writers means operations from multiple clients can modify a shared inode. It does not promise a semantic merge of edits. A read followed by a whole-file replacement can overwrite another writer unless the caller supplies an expected generation. Git operations within the same working tree and arbitrary shell commands need host-level workspace coordination; syscall atomicity is insufficient.

Provide a conditional replace API for agent edits and an exclusive cooperative workspace-operation lease for multi-call workflows. Leases are not a security sandbox and cannot fence an arbitrary process that still has a mount unless storage access is tied to that lease epoch.

## Availability and performance

Default to correctness over availability: if the metadata primary or backend cannot confirm persistence, mutations fail or time out with retry instructions. No offline writes. No promise of sub-millisecond writes or parity with another service. Measure cold/warm reads, small-write object amplification, Git checkout, metadata-heavy directory scans, and pod remount time before setting production SLOs.

One deployment owns a bucket prefix and metadata database. Two independent metadata databases MUST NOT manage the same prefix. Service replicas share one authoritative PostgreSQL primary; database split-brain prevention is an operator prerequisite.
