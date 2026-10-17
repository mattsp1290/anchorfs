# Interfaces and Linux filesystem profile

## Native API

Specify a versioned HTTP API in OpenAPI before implementing clients. Use TLS, bounded binary streaming for file bytes and JSON for metadata. Names, IDs, errors and paths in this project are independent choices. Do not reproduce another vendor's SDK signatures or endpoint naming.

Required operations: workspace create/inspect; stat; list directory; open/renew/close handle; read range; positional write; append; truncate; conditional whole-file replace; mkdir/rmdir; link/symlink/readlink; rename; unlink; mode/owner updates; fsync; mutation-status lookup. Administrative deletion is staged and distinct from unlink; first release may omit workspace deletion rather than risk ambiguous teardown.

Every resource is scoped to workspace and deployment. Validate server-side authorization before resolving supplied inode/handle identifiers. Normal mount users cannot create arbitrary workspaces or set privileged ownership. Use administrator-issued opaque credentials scoped to workspace and read/write roles initially; never expose bucket credentials to clients. Define revocation with a database credential epoch checked during final mutation publication and on each new read/handle operation.

Paths are workspace-relative. Reject NUL, invalid UTF-8 (an explicit first-release limitation), `..`, absolute API paths, and names exceeding 255 UTF-8 bytes; allow paths up to a configurable documented bound, default 4096 bytes. Canonicalize once at the API boundary without double URL decoding. Symlink resolution belongs to filesystem semantics; never follow a user symlink to service-host files. A mount is not a sandbox: hosts must isolate shell processes and enforce mount namespace boundaries.

Pagination must use stable server-issued cursors with scope binding, bounds and expiry. Directory iteration under concurrent change is not a snapshot unless using a pinned view. Never leak another workspace through malformed cursor/handle IDs.

Worker execution capabilities are distinct from host credentials. Only authorized hosts/executors may mint or rebind sessions and acquire workflow leases. Revoking a worker capability fences its existing sessions and rejects new-session/lease acquisition with that capability. See the [lifecycle contract](07-tool-lifecycle.md).

## Identity and permissions

Initial identity model: administrator binds each mount/API credential to one filesystem uid and primary gid; clients cannot assert arbitrary uid/gid or supplementary groups. Pods sharing a workspace normally use the same configured uid/gid. This is a deliberately scoped single-identity mount profile, not unrestricted multi-user Unix identity forwarding.

Server enforces mode bits and directory search permissions for traversal and namespace changes as well as workspace roles. Kernel `default_permissions` is defense in depth, not an alternative to server checks. Apply caller umask to requested creation modes; the authenticated identity owns new inodes. Only inode owner or workspace administrator may chmod; ownership changes require administrator authority. Reject setuid/setgid mode bits and privileged device creation. Select and test sticky-directory semantics before claiming support; initial profile rejects setting sticky bits.

Handles record read/write access flags at open and enforce them on every operation. Ordinary chmod after open follows open-file semantics: previously granted access remains until close/expiry, but credential revocation and workspace-role removal fence existing handles. Direct path API operations recheck current permissions. A read-only credential cannot gain write access via mode changes, guessed handles, or a previously issued writable handle. Persist permission-policy changes transactionally and recheck required authority at publication.

## Exclusive workflows

An exclusive workspace workflow excludes **all mutations**, including ordinary mounts without its token. Acquisition serializes on a workspace authority row and increments its mutation epoch; this fences prior in-flight writes before the grant is returned. Each new mutation captures that epoch and final publication rechecks it. While the lease is active, only its owner/token can publish. Other writers receive a bounded busy/conflict error; readers may continue and are not promised an application-level snapshot.

Renew, release and takeover serialize on the same row. Release/expiry takeover advances the epoch. The mount tied to an expired workflow cannot silently rejoin ordinary mode: it fails closed until explicitly rebound by the host. Ordinary mounts may refresh epochs for new operations after release, but cannot replay an ambiguous old mutation under a new identity. Multi-call workflows are exclusive, not transactional rollback units: a failed Git command may leave its already committed partial work visible.

## Error and timing contract

| Condition | API behavior | Mount mapping |
|---|---|---|
| Invalid input/bounds | 400 with stable code | EINVAL / ENAMETOOLONG |
| Unauthorized/forbidden | 401 / 403 | EACCES |
| Missing entry | 404 | ENOENT |
| Already exists | 409 `exists` | EEXIST |
| Generation conflict | 409 `generation_conflict` | Internal retry for unconditional positional writes; explicit conflict for conditional API edits |
| Expired handle | 410 `stale_handle` | ESTALE |
| No quota/capacity | 507 with code | EDQUOT / ENOSPC |
| Unsupported operation | 501 | EOPNOTSUPP |
| Unavailable dependency | 503 | EIO or ETIMEDOUT, documented per call |
| Ambiguous outcome | 503 `outcome_unknown`, operation ID | Resolve receipt before retry; never report success without evidence |

Define per-call deadlines, cancellation behavior, maximum request sizes, handle/session limits and bounded retry budgets. Cancellation after commit does not undo the operation. Return trace IDs without logging file contents, credential headers or signed URLs.

## Filesystem behavior

Successful write calls persist both data and metadata under the synchronous baseline; fsync is an ordering/durability barrier for all prior successful writes on that handle, not a trigger for asynchronous export. Failed/partial writes report accurate byte counts and errno. Close cannot substitute for checking write/fsync results.

Support O_EXCL atomically with create, O_TRUNC with inode serialization, and rename-over-existing with old open handles retained. Enforce directory/file type errors and nonempty-directory rules. Implement hard links only within a workspace; do not hard-link directories. Initial distributed flock/fcntl requests fail explicitly; do not silently fall back to client-local locks. Database files on this filesystem remain unsupported until their locking/mmap/durability requirements are independently proven.

## Later S3 gateway

The bucket-backed storage adapter is required; an S3-compatible frontend is a separate later milestone. If enabled, it MUST use the same namespace authority, not list private chunk objects as files. Initial target subset: path-style SigV4; HeadBucket; GetBucketLocation; ListObjectsV2; Get/Head/Put/DeleteObject; DeleteObjects; multipart initiate/upload/complete/abort/list. Define operation-specific status/XML and checksum behavior against AWS primary API references during that milestone.

Specify key-to-path rules, file/directory collision handling, single-range HTTP behavior, UTF-8 limits, multipart size limits, atomic complete publication, upload cleanup, and authentication test vectors before shipping. Do not claim generic S3 compatibility from a smoke test. Reject unsupported APIs explicitly. Do not add vendor-specific append semantics by copying documentation; use the native append API. Whole-object PUT and filesystem writes must participate in the same generation ordering.

## Lambda and remote execution

Initial Lambda client direction: native HTTP file operations or the later S3 gateway. The research found a vendor offering remote command execution, not a custom FUSE mount inside Lambda. Do not promise local POSIX paths in Lambda for this service. AWS-managed EFS/S3 Files integrations are separate services and not interchangeable with this object's private chunk layout.

A later remote-job adapter may submit commands to Kubernetes/EC2 workers with the workspace mounted. Specify job IDs, idempotency, cancellation, bounded outputs, execution identity, network/resource policy and isolation before implementation. This is a compute product boundary, not a storage API method that interpolates shell strings. A Lambda `/tmp` copy is only a staging area and cannot transparently provide shared write semantics.

The [tool-worker lifecycle contract](07-tool-lifecycle.md) defines the storage integration for a remote harness and replaceable workers. File reads through the native API need not allocate tool compute; shell execution belongs to a separately authorized execution service.
