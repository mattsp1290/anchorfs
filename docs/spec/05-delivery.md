# Implementation stages and acceptance contract

All stages implement this project's independent specification. Read the provenance rules first. Stages are dependency-ordered deliverables, not claims that implementation or deployment has occurred.

## Stage 0 — Freeze wire and deployment contracts

Write versioned OpenAPI, metadata migrations, object manifest schema and error table. Choose maintained Rust HTTP, PostgreSQL, S3 SDK and FUSE dependencies after checking licenses and platform support. Pin compatible versions; do not copy competitor code. Establish supported Linux/FUSE matrix, limits, credential epoch semantics, retry horizon, and reservation/handle timers. Start with Rust modules for core model, persistence, backend, service, mount and conformance tests; keep protocol handlers separate from storage decisions.

Exit: architecture review resolves ownership of every persistent field and crash boundary; dependency licenses and provenance recorded. No production SLOs are inferred from vendor marketing.

## Stage 1 — Durable single-client core

Implement namespace, immutable chunk upload, generation commit, receipts, authentication, quotas and bounded API calls. Use local test fixtures and a disposable PostgreSQL/MinIO environment. Implement failure injection at upload start/end, precommit, postcommit and response delivery.

Exit: old or new complete committed manifests are visible, never partial ones. Lost responses resolve through receipts without double append/create. Process restart and cache deletion preserve successful writes. Reused idempotency keys with different payloads fail. Oversize, quota, corruption and unavailable-dependency cases do not publish invalid manifests.

## Stage 2 — Shared editing and Linux mounts

Implement the FUSE profile, handles, read pins, atomic namespace operations and cross-client serialization. Exercise two independent client processes, then two pods on different nodes. Namespace locking must cover rename cycles and concurrent directory moves. Add generation-aware conditional replacement and lease-bound workflow mounts.

Exit: tests below pass on real Linux kernels, not only mocked FUSE callbacks. Git smoke workloads pass under explicitly coordinated workflows. Document unsupported locks/mmap and error behavior; do not use Git success as proof of full POSIX compliance.

## Stage 3 — Local and DOKS deployment

Deliver Helm packaging, multi-architecture images, operational docs and host integration examples in Rust and Go. Diagnose current MinIO readiness as an independently authorized infrastructure task before using it. Test provider contracts against actual Spaces with disposable objects and no hidden emulator-only assumptions.

Exit: shared workspace survives pod replacement; node failure evidence is qualified by actual database/backend storage resilience. Restore a database/object recovery set in isolation and verify file bytes/metadata. Demonstrate permission isolation, revocation, quota and draining. Publish measured cold/warm performance and object request amplification without claiming predetermined speedups.

Stage 3 also demonstrates the [harness/worker lifecycle acceptance cases](07-tool-lifecycle.md) using a test orchestrator and disposable workers. This verifies the storage boundary without requiring a production execution service.

## Stage 4 — Optional extensions

After the core gates: EC2/AWS S3 deployment, Lambda HTTP example, optional S3 gateway, remote-job integration and CSI. Each requires its own bounded protocol/acceptance addendum. Snapshot/branching and asynchronous replicated caching require new storage design reviews, not a feature flag over the initial synchronous pipeline.

## Mandatory scenario matrix

| ID | Scenario | Required observation |
|---|---|---|
| T1 | Two clients modify different files | No workspace-wide data-write serialization beyond necessary metadata transactions. |
| T2 | Two clients modify overlapping chunks | Each positional write applies to a current generation; no lost unrelated bytes. Final overlap follows committed ordering. |
| T3 | Concurrent append with response loss | Each operation contributes its bytes exactly once within retry horizon. |
| T4 | Conditional replace races another write | One commit or explicit conflict; no silent stale replacement. |
| T5 | Rename over an open file; unlink-open | New path resolves atomically; old handles preserve old inode until release/expiry. |
| T6 | Cyclic/concurrent directory moves | Valid tree preserved; no orphan namespace or cycles. |
| T7 | Reader caches old contents then remote write | Next new read resolves current metadata under the strict profile. |
| T8 | Process dies after upload, before commit | File remains previous committed version; uploaded object is tracked orphan. |
| T9 | Process dies after metadata commit | Receipt resolves outcome; committed chunks remain readable. |
| T10 | Backend outage or database outage | Bounded errors; no false durable success; no overwrite of unknown outcomes. |
| T11 | Cache lost/corrupt/full | Reads refetch or report backend integrity error; durability unaffected. |
| T12 | Client paused past handle/upload/workflow expiry | Late operation is fenced, not silently resurrected. |
| T13 | Credential revoked during upload | Final commit checks current epoch and rejects unauthorized publication. |
| T14 | Traversal, guessed handles, hostile cursors | No cross-workspace data or host-path access. |
| T15 | Stateful restore | Every manifest reference resolves; open transient sessions invalidated; new writes use restored authoritative state. |
| T16 | Mixed architectures and remount | Same data/manifest format; stable workspace identity after pod movement. |
| T17 | Unsupported operations | Explicit errors for locks/mmap/ACLs/etc.; no fake success. |
| T18 | Retry receipt expiry | Ambiguous operations are not replayed automatically. |
| T19 | Lease acquisition/expiry or handle close during upload | Final publication is fenced; an ordinary writer cannot interfere with an exclusive workflow. |
| T20 | Direct API identity/permission bypass | Enforce traversal, mode, ownership and handle flags without trusting caller uid/gid. |
| T21 | Backup, overwrite/unlink, offline collection, restore | Retained recovery set still restores every referenced byte. |
| T22 | Repeated overwrite and failed upload at tiny quotas | Physical/staging budgets bound growth even when live size is unchanged. |
| T23 | Shrink/regrow within/across chunks and truncate/write race | Newly exposed unwritten ranges contain only zeros. |

Test harnesses must independently assert externally visible data, metadata and receipt outcomes. Add model/property tests for namespace sequences and concurrency histories. Stress tests alone do not prove linearizability; record operations and validate histories for the claimed per-operation guarantees.

## Required handoff evidence from implementing agent

Supply exact revisions, dependency pins, commands/results, provider/kernels tested, deployment topology, measured limitations, unresolved defects, restore evidence and clean provenance records. Never claim Spaces, AWS, Lambda or node-loss acceptance from local emulator success. Do not modify the consumer repositories merely to make a demonstration compile; propose adapter changes separately if needed.
