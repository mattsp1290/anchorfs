# Storage, transactions, and cache coherence

This is an independently selected baseline. PostgreSQL supplies transaction serialization; object storage holds immutable data. No vendor-private algorithm or wire format is required.

## Components and state

- Stateless API replicas authenticate every request and use one PostgreSQL authority.
- Linux FUSE clients translate operations and keep only disposable data caches.
- S3 backend adapter uses explicit endpoint/region/addressing configuration and an established SDK signing implementation.
- PostgreSQL stores workspaces, inodes, directory entries, immutable file manifests, handles, operation receipts, upload reservations, quotas, and deletion candidates.
- Objects use opaque unique IDs under a deployment/workspace namespace. Do not use user paths as object keys. Avoid cross-workspace deduplication in the first version.

Each inode has a monotonically increasing generation, type, size, mode, uid/gid, timestamps and link count. Directory entries have a unique `(workspace, parent_inode, name)` key. A manifest references immutable chunk IDs, lengths and SHA-256 digests. Sparse extents read as zero. Begin with configurable fixed-size chunks, default 4 MiB, as a tuning choice rather than a compatibility guarantee. File-size/chunk-count limits must be explicit and checked for integer overflow.

Truncate serializes against all inode mutations. Shrinking discards logical reachability beyond the new EOF, including the tail of a partial chunk. Rewrite the boundary chunk or encode its valid range. Extension or a far-offset write MUST expose zeros for every newly exposed unwritten byte, never bytes from the old tail.

## Write commit protocol

1. Authenticate, authorize workspace and operation, validate bounds and idempotency key. Create an operation record with request digest and an expiry/heartbeat-protected upload reservation before any object upload. Concurrent attempts with that key must share one operation owner or wait for its receipt.
2. Read an authoritative inode generation and manifest. Materialize only chunks touched by the request; reconstruct untouched bytes in partial chunks. Upload new immutable chunks with unique IDs. Validate lengths/checksums; handle an uncertain upload response by verifying or repeating the same immutable upload, never publishing missing data.
3. Open a short metadata transaction. Lock affected rows in deterministic order, recheck authorization epoch, inode generation, current manifest, quota, and operation ownership. Retry from the new manifest after a concurrent change; conditional requests instead return a conflict. Namespace operations lock the relevant directories and inodes. Also check mount/client-session epoch, handle identity/access mode/closed state/expiry, and workspace workflow epoch/owner/expiry where applicable. Renewals, close, revocation and workflow takeover serialize on those same authority rows; check expiry against authority time immediately before publication. Metadata-only mutations use the same fencing predicates without an upload phase. Retrieving a committed receipt does not authorize a new mutation.
4. Atomically publish the manifest/generation, size/times, quota adjustment, object reachability, and operation receipt. Object persistence MUST precede this transaction's commit. Require PostgreSQL durable commits (`synchronous_commit=on`, `fsync=on`, `full_page_writes=on`) and crash-safe backing storage; replica failover guarantees depend on its configured replication policy. Startup checks must reject unsafe database settings for the durable profile.
5. Reply only after the commit is confirmed. On uncertain database commit, use the operation receipt to determine the outcome. Never blindly repeat an append, rename, or create. Uploads not referenced by a successful commit are garbage candidates, not visible file data.

An API write is bounded (initial maximum 4 MiB). A syscall larger than one API write may produce normal partial-write behavior; it is not a whole-file transaction. Whole-file conditional replacement stages a manifest and publishes it once. Its size limit and upload-session expiry are explicit configuration.

`O_APPEND` chooses the end offset under the same final inode transaction; a generation change requires reconstructing/retrying at the new EOF. Do not reserve an offset and later publish bytes over intervening writes. Append retry returns the original receipt and offset.

## Namespace and handles

Rename is one metadata transaction, including replacement/link counts and directory generation updates. Prevent directory cycles, enforce type rules, reject cross-workspace rename with EXDEV, and serialize ancestor validation against concurrent moves. A conservative per-workspace namespace lock is acceptable initially; document its throughput cost.

Open handles refer to inode IDs rather than paths. Unlink removes the directory entry but retains the inode and manifests needed by live handles. Handles have server-issued identities, expiry, and renewal; client reconnect never silently turns an expired handle into a new file. Bound server restart recovery and return ESTALE for expired handles. Preserve unlinked data while handles or active read reservations pin it.

## Reads and caching

Each read operation obtains a current manifest/generation from the authority and pins it for the request. Fetch immutable chunks, verify digest/length, then serve bytes; never combine chunks from distinct manifests in one read. A multi-call file scan is not a snapshot: clients needing a stable view must use a pinned read session with bounded lifetime.

Cache keys contain deployment/workspace, chunk ID, and expected digest. Cache eviction is always safe; dirty durable state does not exist solely in cache. Bound memory, disk use, in-flight fetches, and read amplification. On a corrupt cache entry, discard and refetch; corrupt backend data yields a surfaced integrity error.

The initial FUSE profile disables kernel data caching for files (direct I/O), writeback caching, and positive/negative attribute/name TTLs. Validate actual kernel/FUSE behavior in multi-client tests. Do not claim this profile supports mmap. Before adding caching, design a proven generation/lease protocol for kernel page and directory caches; notifications alone are not sufficient. API replicas must not serve stale metadata from local caches or asynchronous database replicas.

## Collection and restore

First release: automatic object deletion disabled. Report orphan bytes and quotas accurately; reclaim through an offline maintenance operation that freezes/fences writers across every replica and observes all open/read/upload pins. Preserve objects referenced by every retained recovery set, including chunks no longer in current manifests; alternatively prove each backup has independent copies before deleting source chunks. If backup reachability cannot be established, collection MUST fail closed. Expiring sessions must not allow a paused old operation to commit: final publication checks reservation ownership/expiry in the transaction.

Online GC is a later feature requiring a tested mark/sweep or transactional reference scheme, upload fencing, reader pinning, and backup retention barriers. Age alone is not a safe deletion criterion. Do not enable bucket lifecycle deletion on managed chunks.

Backup metadata and referenced objects as one recovery set. With GC disabled, a consistent PostgreSQL backup plus retained immutable objects can recover its committed manifests; prove this by restoring into an isolated prefix/database with explicit namespace remapping or copied object identities. Record schema/object format versions. Disaster recovery must verify every referenced object before serving writes. Database loss cannot be repaired from opaque chunks alone.

## Capacity accounting

Track separate budgets for live logical bytes/inodes, retained physical object bytes (including old versions and orphans), and in-flight staging bytes. Reserve a worst-case bounded upload allowance transactionally before contacting storage; reject over-budget requests early. Charge failed/conflicting uploads until confirmed deletion, and account each retry object separately. Bound reservations by principal, workspace and deployment; expiry fences publication but does not erase the physical charge. Final commit transfers the reservation into retained physical usage and updates logical usage atomically. Uncertain upload outcomes remain conservatively charged until reconciliation.

Repeated same-size overwrites consume physical capacity even when live size is unchanged. With automatic deletion disabled, the service eventually requires offline maintenance or a capacity increase. At exhaustion return capacity errors without discarding committed data. Alerts must report both logical and physical utilization.

## Retry receipts

Scope keys by workspace and authenticated client identity; store operation type, request digest, committed result, and relevant generations. Reusing a key with different input is an error. Retain receipts for a documented retry horizon (default 24 hours). Bind operation IDs to server-issued bounded client-session epochs; reject old epochs after receipt expiry so a forgotten key cannot become a fresh mutation. Clients must not automatically retry expired ambiguous operations under a new key/session: return an indeterminate outcome requiring inspection. Persistent upload operation records and fencing prevent concurrent retries from publishing twice.
