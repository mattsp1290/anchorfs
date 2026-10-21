# Frozen Stage 0 contracts

Independent design from approved specs 02–05 and 07 and the reviewed Stage 0 plan. Contract freeze means versioned and test-pinned; there are no active users or compatibility shims. Future transaction/runtime behavior remains unverified until the stated stage.

## Stage 1c receipt and manifest foundation (2026-10-20)

Byte operations may retain visible pending receipts while uploading. Their
owner holds the random preimage of the hashed owner token; locking a pending
operation requires that proof. Pending receipts with an active reservation
return `outcome_unknown` and a bounded retry hint. A pending receipt whose
reservations are overdue is fenced on encounter: all possibly uploaded objects
become charged deletion candidates, staging charges are released, and the
receipt becomes failed. Terminal failed receipts return 409 `operation_failed`
(`EIO` hint). Every terminal writer requires the operation still be pending;
a late fence cannot overwrite an acknowledged commit.

Expired workspace receipts become permanent indeterminate/result-null
tombstones. Pending receipts are fenced before tombstoning. Tombstones of
pending/committed operations carry durable `expiry_fences_session=true`; those
of failed operations carry false. After commit, inspection or retry attempts
an idempotent session revocation in a second transaction. Every later encounter
retries the follow-up, so a process death cannot erase it. Administrative
receipts have no session follow-up. Failed receipts remain excluded even after
their result is cleared. An inspecting host may trigger revocation of another
actor's session. Recovery uses an authorized new session; renew/rebind never
revive a revoked session. Tests: `expired_receipt_tombstone_then_session_fence_once`,
`fence_does_not_overwrite_a_committed_receipt`, CP12 and
`expired_receipt_followup_recovers_after_a_crash`.

Every created file has a complete empty manifest at generation 1. Truncate-on-open
publishes a new empty manifest, increments the inode generation and releases
the file's retained logical size; physical objects and old manifests remain.
Read/write handle helpers enforce the granted access flags as well as session
binding and lifetime. These persistence and metadata-route behaviors are tested;
the verified byte methods are described below.

### Stage 1c write pipeline (2026-10-20)

`writeAt` and `append` now use separate snapshot, admission, upload and
publication phases. No database transaction spans a backend request. Receipt
peek precedes handle, file-size, quota and backend validation; the admission
transaction rechecks the receipt for concurrent same-key attempts. It records
an owner proof, reservation and staged objects before the first upload, with
workspace and deployment quota locks last. A same-key pending request returns
`outcome_unknown` with the operation ID; overdue reservations resolve through
the lazy fence. Every PUT error retains a conservative physical charge for all
attempted objects. Only the owner, which tracks never-attempted objects, can
omit those objects from orphan charges. No automatic deletion occurs.

Publication rechecks authority epochs captured at admission, session, workflow,
inode, handle and reservation. It commits objects, a complete immutable
manifest, logical/staging/physical charges and the original result together.
An inode-generation change with the same manifest is a metadata change and
does not require a new upload. A different manifest triggers bounded
re-materialization and retry; append chooses the new EOF. Retry swaps release
the old staging reservation before charging the new one, in the same transaction.
A failed swap rolls back to its savepoint and finalizes the old reservation
and receipt in that transaction. Statement errors are definitive non-publication
and run a separate finalizer. Transport errors, fatal PostgreSQL messages and
connection/unknown-completion SQLSTATEs during COMMIT return outcome_unknown
and preserve the receipt for resolution. An ordinary server error at COMMIT
(such as a deferred constraint rejection) remains definitive and is finalized.
BY3c tests both an actual terminated backend during COMMIT and a deferred
constraint rejection.

The data runner owns a concurrency permit until its detached job finishes.
An atomic Before→Admitting handshake competes with Before→Abandoned at response
timeout: a request answered dependency_unavailable before admission cannot
later begin publishing. After admission, abandonment returns outcome_unknown.
Backend calls have a hard deadline measured from request start; database phases
complete under lock/acquisition bounds, with bounded acquisition retries for
publication/finalization. Lazy quota-pressure fences now receive the configured
lock timeout as well.

Verified against disposable PostgreSQL/MinIO by the byte write suite: persisted
bytes across two sessions' handles, sparse boundary writes, metadata-only
shrink/regrow and same-size truncate, receipt replay after a simulated lost
response and handle close, key/body/offset binding, conservative failed-upload
charges, append conflict retry at the staging limit, and post-upload credential,
epoch and handle fencing. Fsync is a read-mode, receipt-free synchronous-write
barrier returning current inode attributes. ReadRange and replacement acceptance
are described below. The real-binary restart test preserves 6,000 acknowledged
bytes and their generation across SIGKILL and restart against the same disposable
database and bucket; it does not crash the backing containers or storage.

Memory/lifecycle limitations: BinaryBody buffers up to max_request_bytes per
request before database authentication and before a data permit; total buffered
request-body memory is therefore not bounded by max_data_ops. Materialization
is bounded by the data permits, and uploaded buffers are released before retry
preparation. No reservation heartbeat, expiry sweeper, read cache or object
deletion is implemented. The deployment quota singleton and full manifest row
copies remain throughput costs. Uncertain uploads may over-report physical bytes
until future offline reconciliation.

## Byte path

The six byte methods and both pin methods are connected, giving 38 implemented
operations and no stubs (workflow routes added in Stage 2a). ReadRange bounds each call to
1..=max_request_bytes, clips at EOF and returns empty bytes beyond EOF. It
captures one manifest, fetches every touched object whole with digest/length
verification, slices only its logical prefix and zero-fills holes. Corrupt or
missing objects produce dependency_unavailable with backend_integrity and no
retry hint, even when corruption lies outside the requested prefix. There is
no cache. Read amplification is a whole object per touched chunk.

Unpinned X-Anchorfs-Generation is the inode generation observed with the manifest;
pinned reads report that immutable manifest's data generation, which may lag
chmod/link/rename changes. Pins are session-bound, expire, have a per-session
admission bound and can be explicitly released. Create/release use receipts and
read lease mode; pins are permitted during an active workflow. Pin rows and
receipts are never deleted, so churn grows metadata. There is no physical
collection for pins to protect yet.

ReplaceFile requires Content-Length and a 64-hex X-Anchorfs-Content-SHA256.
Receipt-first resolution uses path, expected generation, digest and length;
replay does not poll the supplied body. Nonempty replacement reserves N bytes
before reading, then reads and stages one chunk at a time before PUT. The full
length/digest must match before a single complete manifest is published.
Conditional generation compares the inode strictly, including metadata-only
changes, with no retry. A per-chunk stage transaction costs one round trip per
chunk. Empty replacement publishes atomically without a reservation or PUT.
All body polls and uploads must fit the hard data deadline; this sets the
throughput ceiling for max_replace_bytes (default 256 MiB in 60 seconds).
The stream retains one application chunk, a pending incoming transport frame,
a running hasher and bounded manifest descriptors; it does not aggregate a
whole-file application buffer.

Chunk size is fixed per deployment in schema_meta and compared by bootstrap
and serve. File capacity is min(default max_file_size, chunk_size times
max_chunks_per_file). Writes/truncates/replaces reject mismatched manifest
geometry; reads honor the recorded geometry. Shrink clips logical reachability
without PUT, and regrowth or a later write exposes zeros instead of discarded
tails. Manifests and old objects are retained, so full manifest row copies and
old versions consume metadata/physical capacity.

Accounting uses live retained inode sizes for logical bytes, active reservation
sums for staging, and published object lengths plus deletion-candidate bytes
for physical usage. Workspace and deployment staging/physical changes share
the same transaction; the deployment singleton serializes those updates.
Expired reservations release staging only by converting every possibly uploaded
object into a retained physical charge. A confirmed absence is not inferred
from a PUT error; conservative orphan over-reporting remains until future offline
reconciliation. The invariant checker and independent bucket listing verify
these formulas against disposable PostgreSQL/MinIO.

### Confirmed interpretations of spec 02

The owner confirmed the following interpretations in the approved plan on
2026-10-19: a concurrent same-key pending request returns outcome_unknown rather
than waiting (W16); quota pressure lazily fences overdue reservations instead
of running a sweeper (W16b); max_reservation_bytes is a per-request principal
bound rather than an aggregate (W33); reservation heartbeat refresh is deferred
and TTL is bounded against backend/lock/acquisition deadlines (W25); pins are
reader-side state allowed during leases (W34); receipt expiry revokes sessions
for pending/committed outcomes, including when another actor inspects them,
with repeatable follow-ups and a failed-receipt exclusion (W35).

The engineering refinements and dependency exception are recorded in the
[implementation amendments](../provenance/stage1c-implementation-amendments.md).
The sequential byte model also checks mixed operations against a dense in-memory
oracle. These tests do not establish the remaining full failure/concurrency
matrix, Linux mounts, Spaces/AWS support, or backing-storage durability.

## Limits

Values are bytes or counts, with file size 4 TiB derived from checked chunk_size × max_chunks_per_file. Core validation bounds PostgreSQL integer/bigint columns and rejects zero/overflow. API bodies and whole-file replacement have separate caps.

| name | value | source | status |
|---|---|---|---|
| chunk_size | 4194304 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_api_write | 4194304 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_replace_bytes | 268435456 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_chunks_per_file | 1048576 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_file_size | 4398046511104 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_name_bytes | 255 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_path_bytes | 4096 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_list_page | 1000 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_handles_per_session | 4096 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_sessions_per_credential | 64 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_read_pins_per_session | 64 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |
| max_retry_attempts | 3 | spec 02/03; independent default | verified: contracts_limits_match_defaults (values only); runtime semantics Stage 1 |

## Timers

Seconds. The repository owner accepted the U2 defaults on 2026-10-17 at the Stage 0 sign-off. All future operational timers must be configurable; Stage 0 only exposes the service deadlines.

| name | value | source | status |
|---|---|---|---|
| handle_ttl | 300 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| upload_reservation_ttl | 900 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| reservation_heartbeat | 60 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| client_session_ttl | 3600 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| workflow_lease_ttl | 600 | spec 02/03; independent default | configurable: ANCHORFS_WORKFLOW_LEASE_TTL_MS; lifecycle verified by WL-N1–N7 |
| read_pin_ttl | 300 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| receipt_retention | 86400 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| cursor_ttl | 600 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| restart_recovery_bound | 60 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| metadata_call_deadline | 10 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| data_call_deadline | 60 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| shutdown_drain | 10 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |

## Transaction model

READ COMMITTED, explicit locks and the complete order/modes in [persistent-state.md](persistent-state.md). PostgreSQL clock_timestamp() is the expiry authority, transaction_timestamp() supplies recorded times, and each pool connection sets synchronous_commit=on. The commit phase from Stage 1 runs in a spawned non-cancellable task: disconnect/deadline cannot undo publication; uncertain waiting yields outcome_unknown; clients resolve by idempotency key when no operation ID is known. Stage 0 stubs never begin mutations; a deadline returns dependency_unavailable with bounded retry guidance. Control-plane transaction behavior is verified by `control_plane` CP4, CP5 and CP13; native file publication is exercised by the Stage 1c byte suite, with process-restart evidence in `serve_restart_preserves_bytes`. The complete acceptance matrix remains in progress.

## Credential epochs and sessions

Credential rotation/revocation increments that credential's epoch; role/permission-policy changes increment workspace authorization_epoch. Capture both at operation start and recheck at publication. admin_operations stores credential_epoch_at_start; operations additionally stores workspace and session epochs. New read/handle operations check current authority. Issuer revocation cascades to issued credentials and their sessions. Read/write/admin roles retain server permission checks; only owner or workspace administrator may chmod, ownership changes require administrator. Workers cannot issue replacement sessions or acquire leases. Credential/session behavior is verified by `control_plane` CP7–CP12; filesystem permissions are verified by Stage 1b; lease routes by Stage 2a WL-N1–N7 and WL-B1–B3.

Receipt keys are workspace + authenticated credential + idempotency key, with digest binding and 24-hour retention; deployment/session-minting routes use admin_operations. An expired ambiguous operation returns operation_indeterminate; it must not be automatically replayed under a new key/session. max_retry_attempts bounds retries. Client sessions are bounded and receipt replay cannot grant a new mutation. Control-plane receipts are verified by CP4, CP5 and CP12.

## Exclusive workflows

The workspaces authority row stores workflow owner/token hash/expiry. Stage 2a
implements the four routes and session binding, verified by namespace WL-N1–N7,
byte WL-B1–B3 and the persistence schedules in the acceptance map.

| Transition | Required recorded state | Result |
|---|---|---|
| Acquire | absent | epoch +1, new owner/token/expiry |
| Renew | live, owner with valid proof | same epoch/token, expiry extended |
| Release | live, owner with valid proof | epoch +1, lease fields NULL |
| Takeover | expired | epoch +1, new owner/token/expiry |

Only write/admin-role hosts with ordinary, unbound actor sessions may change
leases. A recorded live lease blocks acquire/takeover (`workspace_busy`,
`lease_active`); an expired recorded lease blocks acquire (`workspace_busy`,
`lease_expired`). Takeover and renew/release on an absent lease return
`workspace_busy` / `lease_absent`. Only the owner may renew/release (otherwise
403); an expired owner's proof returns `fenced` / `lease_expired`. Wrong proofs
return `fenced` / `workflow_proof_mismatch`; bound actor sessions return `fenced`
/ `session_bound`. Administrators and workers cannot hold leases.

An owner host may mint or rebind worker/host sessions with the token. Existing
actor/target checks precede binding. Under a live lease, mutations require
proof from the owner or a session bound to the current epoch. Release/takeover
leave old sessions persisted and fence their reads and mutations until a host
rebinds them. Any host may rebind to ordinary mode, which grants no ability to
publish through someone else's live lease. Expiry is lazy, with no sweeper.

Stage 2a interpretation (2026-10-20, approved plan): mere expiry does not advance
the epoch. A bound session keeps reading until release/takeover advances it;
publication fails closed immediately on expiry. Readers are never promised
exclusion or application-level snapshots. `check_workflow` is unchanged.

`ANCHORFS_WORKFLOW_LEASE_TTL_MS` is configurable, defaults to 600000, and must
be positive. Renew uses database wall time plus the configured TTL and never
rotates or returns a token. Workspace inspection returns owner, epoch, expiry
and active status, with no token/hash. See the Stage 2a acceptance map.

Lease routes preauthenticate kind/role/workspace without a row lock, then take
workspace FOR UPDATE before credential/session/receipt locks; they never
upgrade FOR SHARE. The exclusive lock timeout is bounded by the remaining
response budget (minimum 1 ms). There remains a small post-lock/commit window
where an abandoned caller can lose the initial grant; recovery requires expiry
and takeover. PostgreSQL scheduling under sustained shared traffic has no
promised acquisition latency: the local `acquire_under_publication_load` run
observed 113 ms, which is an observation, not a bound or starvation proof.
A revoked owner cannot renew; its lease persists until expiry/takeover, and
issuer revocation cascades to its workers under the existing credential rules.

## Object format and deployment identity

OBJECT_FORMAT_VERSION=1: an object is raw bytes of one chunk without framing. Its independent opaque key is deployment_id/workspace UUID/object UUID. SHA-256 lives in objects and manifest_chunks with a composite digest FK. The manifests/manifest_chunks tables define manifest format. Absent indexes and bytes beyond logical_length read as zeros; shrink/regrow must never expose old discarded tails. Manifest updates are rejected by schema triggers (verified: manifests_immutable); object persistence and verified native reads are exercised against disposable MinIO by the Stage 1c byte suite; provider storage durability remains unverified.

One deployment_id is both schema_meta identity and bucket prefix. Startup compares stored identity and configuration (verified: migrations_apply_revert_reapply). Two independent metadata authorities must never own the same prefix. Physical, staging and logical quota accounting are implemented and checked by Stage 1c workspace/deployment/bucket oracles. Offline collection remains future work; automatic deletion is disabled.

## Open deviations

TLS (U1): spec 03 requires TLS. Stage 0 listener is plaintext, restricted to loopback under the durable profile; development may bind all addresses. TLS_CERT_PATH/TLS_KEY_PATH are reserved and rejected. TLS termination is a Stage 1 decision. Verified boundary: serve_refuses_non_loopback_plaintext_durable.

Readiness lists one key under the deployment prefix through a retry-free client, with a bounded deadline and a one-second single-flight memo. It checks reachability, authentication, bucket existence and list permission; write permission, durability and replication remain unprobed. The service readiness setting `backend_probe_deadline` is deliberately outside the storage Timers table. TLS U1 and the development-only plaintext S3 rule are unchanged, so durable-profile end-to-end successful backend readiness remains unverified until U1. PostgreSQL reset_val plus role/database override checks detect the specified unsafe profiles; a privileged role setting can mask a lower-level server default, so these checks do not certify every deployment configuration or storage hardware.

## Linux/FUSE matrix

Official platform references consulted 2026-10-17. This is a target matrix, not a runtime compatibility claim.

| Target | Contract | Source | Status |
|---|---|---|---|
| Linux kernel | ≥5.10 with CONFIG_FUSE_FS | https://docs.kernel.org/filesystems/fuse/fuse.html | observed on Linux 6.8.0-124-generic; full Stage 2b gate in progress |
| FUSE protocol | ≥7.31 | https://github.com/torvalds/linux/blob/v5.10/include/uapi/linux/fuse.h | observed on Linux 6.8.0-124-generic; full Stage 2b gate in progress |
| mount helper | fusermount3, fuse 3.x | https://github.com/libfuse/libfuse | observed on Linux 6.8.0-124-generic; full Stage 2b gate in progress |
| Strict file profile | direct_io=true, writeback_cache=false, allow_mmap=false | spec 02 Reads and caching | serialization and Linux 6.8.0-124-generic kernel tests passed; final gates in progress |
| Metadata cache | attr_ttl=entry_ttl=negative_ttl=0 | spec 02 Reads and caching | serialization and Linux 6.8.0-124-generic kernel tests passed; final gates in progress |
| Permissions | default_permissions=true; server checks mandatory | spec 03 Identity and permissions | server conformance and Linux 6.8.0-124-generic permission tests passed |
| Other OS | CLI prints profile; mount returns UnsupportedPlatform | independent plan D5 | macOS stub tests verified; other targets unverified |

The Stage 2b options are default_permissions, nosuid, nodev, noatime, fsname=anchorfs and subtype=anchorfs. Direct I/O is an open/reply flag and TTLs are reply fields; auto-cache and writeback remain disabled. Shared writable mmap, flock/fcntl, ACLs, xattrs, device nodes, sockets/FIFOs and renameat2 flags beyond none are unsupported. The Linux mount profile below records the implemented translation and limits.

## Provider matrix

| Provider | Status |
|---|---|
| MinIO local | partially verified: `anchorfs-conformance/tests/backend.rs` F1–F15 against `bitnamilegacy/minio:2025.7.23-debian-12-r5@sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20`, single node, plaintext loopback. Delete excluded. Throttling, timeouts and lost responses are simulated by an in-test proxy. Checksum enforcement observed through one raw SDK request (F11b). No multi-node or durability claim. |
| DigitalOcean Spaces | unverified until Stage 3 live disposable contract suite |
| AWS S3 | later stage; unverified |

No object-store durability, FUSE runtime, provider compatibility or performance claim follows from Stage 0 builds, schema tests or SDK construction.

## Reviewed wire decisions

These are independent requirements/design clarifications from the Stage 0 code reviews; imported baselines remain unchanged. Future success responses are declared per operation, and Stage 0 initially returned 501 after shape validation. Stage 1a implements the thirteen control-plane routes listed below. Grant expiry strings are RFC3339 UTC timestamps. MutationResult carries operation_id and, when relevant, the affected inode/generation; WriteResult requires offset, bytes_written and committed generation (including the original offset on append retry). OperationStatus.result contains the original success metadata while retained, with secret/token fields omitted; an expired tombstone has state=indeterminate and result=null. Credential secrets and newly issued workflow tokens are returned by the initial grant response, never stored as plaintext in a receipt or returned by inspection. Stage 2a amendment (2026-10-20): renewal returns token:null without rotation. Credential/workflow grant replays return committed metadata with secret/token:null, matching the Stage 1a credential behavior. A replayed grant is not a usable secret; inspection still proves committed metadata. Acquire/takeover workflow headers must be absent (invalid_input / workflow_proof_not_applicable). A lost issued-credential secret is recovered by authorized rotation after inspection, and a lost workflow token requires waiting for expiry and authorized takeover (up to workflow_lease_ttl); neither case repeats the original mutation. Stage 1 must test these recovery paths before implementing grants.

For stat/list, an omitted query path selects the workspace root; for setAttributes an omitted JSON path selects root. Any supplied path remains nonempty and relative: empty, absolute, dot/traversal and invalid UTF-8 paths are rejected. Creation/removal targets still require a child path.

All workspace operations except getWorkspace, createSession and receipt inspection require X-Anchorfs-Session-Id (UUID) and X-Anchorfs-Session-Epoch (positive integer). This names the actor session; a session ID in the URL names the target session and does not replace actor authority. Authorized hosts first mint an ordinary actor session to acquire/take over a lease, then mint/rebind worker sessions to its grant. A bearer credential alone cannot select one of its multiple sessions. Deployment operations and session creation use admin_operations and require no preexisting actor session. Receipt inspection uses current authenticated original-credential scope, not an expired session.

Workflow authority is the paired X-Anchorfs-Workflow-Token (64 hex characters encoding 32 random bytes) and X-Anchorfs-Workflow-Epoch (positive mutation epoch). Both must be present or both absent. Renew/release require them. During an active lease, every mutation must present its current grant; ordinary operations may omit it only when no active lease applies. Bound sessions also record the grant epoch and fail closed after expiry until authorized host rebind. Stage 0 validates header shape only; Stage 1 must verify token hash, original credential, actor/target binding, current epochs/expiry and workflow/session authority under transaction locks.

GET /v1/admin-operations/{operation_id} and GET /v1/admin-operations?idempotency_key=... inspect original-credential-scoped administrative receipts. After a lost createWorkspace response, the authenticated original issuer can look up its original key without knowing either generated workspace ID or operation ID. It must not create again under a new key. Expired results return the retained tombstone through inspection and operation_indeterminate on mutation retry. B11 in persistent-state.md defines permanent identity/key retention and atomic session fencing; session renewal never revives a revoked epoch.

## Backend integrity contract

PUT verifies the local SHA-256 before uploading a nonempty object of at most the default chunk size. The default `checksum_mode=sha256` sends a signed SHA-256 checksum header: a matching provider echo confirms it, an absent echo triggers verified read-back, and a wrong echo is an integrity error. Mode `none` relies on transport integrity for successful uploads. SDK response checksum validation is disabled; AnchorFS verifies digest and length in `get_verified`. `get_range` verifies Content-Range and Content-Length framing only, and surfaces absent or clamped ranges rather than zero-filling. Both read forms share one configured request deadline across response headers and complete body consumption, including verified GETs used by upload read-back.

Ambiguous uploads reconcile through HEAD and, when needed, verified GET, repeating the same immutable bytes only after observed absence. The bound is two PUT, two HEAD and two GET calls, each with bounded SDK retries. `Unavailable` after reconciliation means not observed stored after the repeat and final HEAD; a delayed request may still land. Failed observation yields `UncertainOutcome`. Control-plane metadata and Stage 1c byte publication/receipt operations are implemented; see the [byte path](#byte-path) and [acceptance map](../provenance/stage1c-byte-path-acceptance.md). No Spaces/AWS compatibility or performance claim follows from local provider tests.

## Control plane authority

Stage 1a, 2026-10-18: independent implementation from approved specs 02, 03 and
07 and the reviewed local plan. This amendment changes local design records;
imported specification baselines remain unchanged. Bearers are
`<credential UUID>.<64 lowercase hex secret>` with a case-insensitive `Bearer`
scheme. The stored value is SHA-256 of the secret's UTF-8 bytes. Authentication
checks the digest, revocation and expiry before locking, then checks again under
a row lock. Missing/malformed/unknown/expired/revoked credentials return 401;
`WWW-Authenticate` is bare `Bearer` when absent and `Bearer error="invalid_token"`
when a token was presented. Invalid path/body/key shapes fail before database
authentication; the bearer grammar extractor precedes JSON bodies.

A = deployment administrator, H = workspace host, K = worker; W is the actor's
workspace. Missing/out-of-scope targets return 404 after authentication.

| Operation | A | H | K |
|---|---|---|---|
| `bootstrap-admin` | CLI only; not an HTTP route | — | — |
| `createWorkspace` | allowed | 403 | 403 |
| `getWorkspace(W)` | allowed | allowed; other → 404 | allowed; other → 404 |
| `createCredential` kind=administrator | allowed (`workspace_id` must be null and role admin, else 400) | 403 | 403 |
| `createCredential` kind=host | allowed (`workspace_id` required; role read/write/admin, else 400) | 403 | 403 |
| `createCredential` kind=worker | allowed, any uid/gid, role read/write, any lifetime and allowance | allowed only when body `workspace_id` equals H's workspace (null → 400; other → 404), `uid`/`gid` equal H's, role ≤ H's role (read ≤ write), `max_reservation_bytes` ≤ H's, and `expires_at` ≤ H's `expires_at` (a `null` `expires_at` is allowed only when H's is `null`); otherwise 403 (D13) | 403 |
| `rotateCredential(X)` | allowed | X == H (self), or `X.issued_by == H` and X in W; X not in W or unknown → 404; in W but not issued by H → 403 | 403 |
| `revokeCredential(X)` | allowed for any X, including A's ancestors and A itself (the response commits; A's later requests → 401; recovery is another administrator or `bootstrap-admin` once no unrevoked administrator remains) | `X.issued_by == H` and X in W; H itself → 403; not in W or unknown → 404 | 403 |
| `getAdminOperation`, `findAdminOperation` | own receipts only | own receipts only | own receipts only (workers can hold none) |

| Operation | A | H (actor session in W) | K (actor session in W) |
|---|---|---|---|
| `createSession(W, target T)` | T in W, `T.kind ∈ {host, worker}`, not revoked/expired; other workspace, unknown or revoked → 404 | same; T may be H itself | 403 |
| `renewSession(W, S)` | — | any S in W | S == actor session only; otherwise 403 |
| `rebindSession(W, S)` | — | any S in W, body `workflow_lease_token` null | 403 |
| `revokeSession(W, S)` | — | any S in W | S == actor session only; otherwise 403 |
| `getOperation(W, id)` | `same_workspace` (A always) and (`operation.credential_id == actor` or actor is H of W); else 404 | same | own receipts only |
| `findOperation(W, key)` | by `(W, actor credential, key)`; A has none → 404 | own | own |

`bootstrap-admin --uid <uid> --gid <gid>` serializes first-administrator creation
and refuses while any unrevoked administrator exists. It needs only metadata
configuration. Operators who lose the sole unrevoked administrator secret must
explicitly revoke that row through an authorized database maintenance session
before bootstrapping again; bootstrap never resets a live administrator.

CreateWorkspace requires `root_uid` and `root_gid`; `root_mode` defaults to
0755 and rejects special bits. Root generation is 1 and link count is 2. Limits
must be positive and fit PostgreSQL bigint. IssuedCredential.secret is optional
and serialized as null on replay. Fresh issuance/rotation responses alone carry
the secret and `Cache-Control: no-store`. Receipts never store plaintext secrets.
A lost self-rotation response needs a new rotation by an administrator or issuing
host; the stale proof cannot inspect or replay. Rotation increments credential
epoch and revokes sessions, releasing their active slots. Host-issued worker
identity, role, allowance and lifetime cannot exceed the host's grant.

Administrators and hosts mint sessions; workers cannot. Session limits count
unrevoked/unexpired sessions and serialize under a per-target transaction advisory
lock. Rebinding an expired session consumes a slot under the same lock; an
already active target can rebind at capacity without consuming another slot. Every session mutation checks the actor's credential, session ownership,
workspace, expiry and both epochs. Workers renew/revoke only their actor session;
hosts manage any session in W. Rebind is host-only, permits expired targets,
increments the session epoch and clears binding. Revoked sessions stay terminal.
Workflow-lease request fields remain 501 before database access.

Receipts bind the operation, path IDs and recursively sorted canonical JSON using
SHA-256 over `anchorfs-request-v1\0<op>\0<path IDs joined by '/'>\0<body or null>`.
Administrative receipts scope by actor credential/key; workspace receipts also
scope by workspace. Receipt insertion uses ON CONFLICT DO NOTHING, waits for the
owner within the remaining budget, then reselects its result. Same digest replays
committed metadata; different digest returns 409 idempotency_mismatch with the
operation ID. Business errors roll back the receipt and mutation together.
Self-fencing mutations may reject a replay at authentication/session verification;
the client needs current authority or a replacement session.

Expired receipts become permanent indeterminate/result-null tombstones, including
through inspection, which commits that transition before responding. Retry returns
409 operation_indeterminate with the operation ID. Stage 1c implements lazy
session revocation in a separate idempotent post-tombstone transaction (W35),
as amended in persistent-state.md. Find-by-key
is always actor-scoped; hosts can inspect workspace receipts by ID, administrators
cannot inspect another credential's workspace receipt. Inspection never grants
authority.

The request budget begins in trace middleware. The runner reserves 200 ms,
refuses when at most 100 ms remains, acquires an owned concurrency permit, then
spawns a transaction that keeps its permit until completion. Client abandonment
cannot cancel publication. Mutation budget exhaustion, task panic or any COMMIT
error returns 503 outcome_unknown; clients resolve by key. Reads and admission
failures return dependency_unavailable. A 30-second configurable lock timeout
bounds each ordinary lock wait; receipt insertion temporarily lowers it to the
remaining budget. 40P01/40001/55P03/57014 before commit map to retryable 503
with 100 ms guidance; session limit returns 507 quota_exceeded. The service does
not retry internally. No rate limiting or background expiry sweeper exists.

Stage 1a review correction (2026-10-18): row locks preserve authority fields but
cannot freeze time. Every implemented mutation checks its originally admitted
credential and required actor-session expiry again at the final publication
boundary after receipt, target and admission waits. Captured original expiry is
used even for self-renew/rebind/revoke; reloading the mutated epochs would wrongly
reject legitimate self-fencing results. Minting also rechecks target-credential
expiry after admission/insert waits. Expiry rejects and rolls back state plus
fresh receipt (401 credential, 409 actor-session fence, 404 expired mint target).
Receipt replays and inspection recheck current admitted credential time before
returning metadata. This adds enforcement of the existing spec 02 predicate;
it does not change the imported baseline.

Stage 1a admission recheck (2026-10-18): renew also holds the same per-credential
admission lock through COMMIT. Without this, another transaction could count an
old committed expiry after it lapses while a renewal's extension is uncommitted,
admitting a replacement before the renewal becomes visible. Target expiry is
checked after acquiring admission, so renew still fences an expired target;
rebind remains the explicit reactivation path. The lock covers create, renew and
rebind, and a blocked-minter regression verifies the MVCC interval.


Stage 1a service-boundary consolidation (2026-10-18): success responses are
constructed from the declared wire DTOs, including explicit persistence enum and
numeric conversions. Heterogeneous durable receipt payloads remain JSON. The
service's `receipt::execute` owns receipt dispatch, the final original-authority
time check and fresh receipt storage. `Published::credential` derives a secret-free
stored DTO and a fresh response from the typed optional secret, which also selects
no-store; no response-shape probing is used. Route callbacks keep their authority
predicates, required locks and business mutation explicit. Persistence helpers
still check their own publication work for direct callers; callers adding further
waits must guard their final publication point. Receipt lookup is an Id/Key enum,
with by-key lookup always actor-scoped and host by-ID visibility unchanged.

### Stage 1b authority amendment — 2026-10-18

Deployment administrators may issue host credentials with role admin. These are the workspace administrators of spec 03 and bypass mode checks; workers remain read/write only. Hosts may issue only workers and cannot delegate admin. Deployment administrators have no actor sessions and cannot perform namespace operations directly. Workflow token hashes use SHA-256 of the 32 decoded token bytes; mutation proofs require the lease owner credential or a session bound to the current lease epoch. Bound sessions fail closed after lease expiry until explicitly rebound.

## Namespace and handles

Stage 1b amendment — 2026-10-18. The native API implements 26 operations: the thirteen Stage 1a control-plane routes, ten namespace routes and three metadata-only handle routes. Twelve byte/read-pin/workflow-lease operations still return 501 after shape validation. No namespace or handle operation calls object storage.

Workspace mutations publish inode, entry, logical quota and receipt changes in one PostgreSQL transaction. Receipt scope includes workspace, credential and captured session/authority epochs. Same-key identical retries return the original operation ID and metadata; a different body returns idempotency_mismatch. Expired receipt identity is retained and retry returns operation_indeterminate. Open-handle replay returns its stored grant even if subsequently closed or expired; renew/close reveal current state. Role and workflow gates apply before receipt dispatch, including replay. Stage 2a exception (2026-10-20): lease routes gate kind, role, workspace and session_bound before receipt dispatch; lease state, ownership and proof checks run only for fresh mutations, so replay returns committed metadata without reevaluating those checks; findOperation inspects a committed receipt while writers are excluded by a lease.

A workspace namespace guard serializes entry changes, ancestor validation and metadata changes. The lock order is the dated Stage 1b amendment in persistent-state.md: authority → receipt → namespace → ascending inode IDs → entries → quota → handles → admission. Renew takes inode → handle → session-keyed advisory admission lock 7002. Every changed entry increments namespace_generation once per API operation; chmod/chown, existing-file open, truncate-empty and no-op rename leave it unchanged. Parents gain generation and mtime/ctime on entry changes; modified target inodes gain generation and ctime. Directory nlink is two plus live child directories (zero after removal); file/symlink nlink equals its entry count. Rename replacement adjusts each parent's child-directory count exactly once, and prevents cycles. A same-path or same-inode rename still resolves/checks authority, then does not change generations.

Paths never follow symlinks: an intermediate symlink is not_directory, final symlinks are returned as symlink inodes. Targets are opaque UTF-8, nonempty, NUL-free and at most 4096 bytes. Symlink mode is fixed at 0777; chmod on symlinks returns invalid_input. Creation uses the authenticated uid/gid and requested final mode. **Deviation from spec 03's caller-umask wording:** the future mount applies umask before sending the requested mode; no umask field exists on credentials or requests. Special bits above 0777 are rejected. Owner/group/other class selection uses one uid and primary gid, with owner class taking precedence. Search is required on traversed directories; namespace parents require write+search; listing requires read+search. Stat requires search on ancestors but no permission bit on the target. Admin-role hosts bypass mode bits and can chown; they do not bypass quotas, type rules or cycles. Deployment administrators cannot perform namespace operations without an actor session, which they cannot mint for themselves.

List cursors use `1 || hex(canonical JSON payload) || hex(HMAC-SHA256 tag)`. Payload fields are workspace, parent_inode, last_name, namespace_generation and expires_unix_ms. The 32-byte hex-decoded signing key authenticates `anchorfs-cursor-v1 || payload` using RustCrypto hmac; MAC verification precedes parsing. Expiry uses database clock milliseconds. Bad MAC/shape, expiry or scope returns 400 invalid_input with message cursor_invalid, cursor_expired or cursor_scope. C-collated keyset order continues after last_name; namespace generation changes are reported, not rejected. Reads and non-create opens walk READ COMMITTED state without a namespace lock; a racing rename/chmod can return 404 or evaluate an earlier directory's permission. Listings and multi-step walks are not snapshots.

Logical quota counts each non-root inode once plus its size (symlink target bytes here); hard links add no charge. Unlink/replacement retains the tombstone inode and charge while unexpired open handles remain. The only release helper sets charge_released_at atomically with the quota decrement when nlink=0 and no live handle remains. Close releases the last live pin. Expired handles cannot be renewed or closed; charges pinned by lazy expiry remain until a later sweeper or a subsequent eligible operation. No inode rows are deleted.

Limits: no byte routes or manifests, and truncate-on-open of an inode with a manifest is 501 until the byte milestone. Workflow routes remain 501; only the owner/token/epoch and bound-session predicates are implemented and exercised with disposable fixture lease rows. Maintenance state is currently ignored. Expiry sweepers and physical tombstone bounds are deferred; repeated create/unlink can grow inode rows at zero live logical charge. Development signing keys are ephemeral, so restart or differing replica keys invalidate cursors; clients restart listing. Durable deployments require a configured key. These behaviors do not claim full POSIX compliance, competitor compatibility, mount support or provider durability beyond the tested local contracts.

Publication-time clarification — 2026-10-18: namespace and handle receipt jobs carry the admitted workflow context to the common final validation after all business/receipt waits. Mutation-mode authority must still have a live matching owner/bound-session proof immediately before publication. Read-mode renew/close continue during lease expiry; self-fencing control-plane jobs have no workflow context. Handle renewal takes inode → handle → admission, then conditionally extends only if the original persisted expiry is still in the future. Crossing expiry while waiting for admission returns stale_handle, preventing renewal from reactivating a slot already admitted to a different open.

## Explicit inode times (Stage 2b, 2026-10-21)

`setAttributes` accepts optional RFC3339 `atime` and `mtime`. Explicit times
require the inode owner or a workspace administrator and are permitted on files,
directories and symlinks. Omitted times are unchanged; stored instants have
PostgreSQL microsecond precision. Each successful update advances generation
and sets `ctime` to transaction time, so conditional replacement observes a
conflict after a timestamp change. Years outside 1 through 9999 are rejected.
This is an independent extension for the approved mount's `utimens` path.
A non-owner writer cannot touch a file using this extension. `UTIME_NOW` in
the mount uses client wall time; `ctime` continues to use the server clock.

## Linux mount profile (Stage 2b)

The native client and Linux FUSE mount implement the strict profile. Kernel
acceptance is recorded separately in `docs/provenance/stage2b-linux-mount-review.md`;
other kernels, cross-node mounts and provider durability remain unverified.
Server inode ids are FUSE inode numbers except the workspace root, mapped to 1.
Entry and attribute TTLs are zero; opens select direct I/O without keep-cache,
writeback-cache or shared-mmap capabilities. Reads do not update atime.

Lookup and path mutation callbacks reconstruct names and check the expected
parent/target inode identity before requesting a mutation. Replies with inode
identities are checked again. A time-of-check/time-of-use window remains between
that check and the server's path resolution. Open file operations use server
handle ids. When Linux omits fh from getattr, the mount can use another live
handle for that same inode, which preserves fstat after unlink. Getattr through
an open handle costs a receipt-free fsync round trip. Directory handles retain a
paged listing snapshot; readdir does not populate the inode lookup table.

Writes commit synchronously in bounded API slices. O_APPEND selects server EOF
for each slice. Ambiguous outcomes retain their key and resolve their original
receipt; only definitive generation conflicts retry under new keys, at most
three times. A partial write reports only bytes known committed. An unresolved
slice reports EIO and its operation id is logged when available.

Live workflow authority requires a host-minted bound session and the paired
workflow token/epoch. The mount never renews or releases workflow leases. A
fenced mount returns ESTALE until its process is replaced; a host rebind alone
does not clear the latch. Workspace contention returns EBUSY without latching.
Ordinary mounts probe root after a fenced operation so a publication race does
not destroy a still-valid session. A bound mount latches a fenced mutation
except reservation_expired even when the server still permits reads after
mere lease expiry. A post-startup 401 maps to ESTALE and latches, an explicit
deviation from spec 03's EACCES row. A stale handle fences that handle.

The session and handles renew using local monotonic intervals derived from
operator-supplied TTL hints; the hints must match the service. Renewal is best
effort and a long pause still fences. Session identity and workflow epoch are
validated at startup. Worker credentials need a pre-minted session; a host
credential can self-mint. Clean shutdown revokes only self-minted sessions.
SIGTERM/SIGINT first attempt a non-lazy unmount; if tools keep it busy the mount
continues serving with its handles intact. Forced process death leaves its
self-minted session to expire. Flush returns success for synchronous writes,
including after fencing, so close does not invent a second write error.

Kernel default_permissions and server authorization both apply. The mount
assumes its euid/egid and the authenticated workspace identity are aligned;
allow-other is an explicit opt-in and needs host FUSE permission configuration.
Setuid/setgid/sticky bits and special file nodes are rejected with EPERM.
Flock and fcntl locks are forwarded and rejected with ENOLCK; initialization
fails if the kernel cannot forward them. Xattrs and other unsupported callbacks
return ENOSYS (the kernel may expose EOPNOTSUPP); renameat2 flags return EINVAL.
Shared writable mmap is unsupported. Private pages mapped before fencing may
remain readable; Git success is a smoke observation and makes no mmap or
POSIX-compliance claim. Statfs reports zero capacity, a deliberate limitation.

Credentials and workflow tokens come from files or environment, never argv.
The client refuses plaintext without allow-plaintext, disables environment
proxies and redirects, and validates TLS using rustls with optional PEM roots.
The https/CA-bundle path is unexercised. JSON logs stay at info to avoid dependency
debug events containing names; local text logging may opt into debug.
