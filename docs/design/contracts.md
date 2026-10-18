# Frozen Stage 0 contracts

Independent design from approved specs 02–05 and 07 and the reviewed Stage 0 plan. Contract freeze means versioned and test-pinned; there are no active users or compatibility shims. Future transaction/runtime behavior remains unverified until the stated stage.

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
| workflow_lease_ttl | 600 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| read_pin_ttl | 300 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| receipt_retention | 86400 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| cursor_ttl | 600 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| restart_recovery_bound | 60 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| metadata_call_deadline | 10 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| data_call_deadline | 60 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |
| shutdown_drain | 10 | spec 02/03; independent default | verified: contracts_timers_match_defaults (values only); lifecycle Stage 1/2 |

## Transaction model

READ COMMITTED, explicit locks and the complete order/modes in [persistent-state.md](persistent-state.md). PostgreSQL clock_timestamp() is the expiry authority, transaction_timestamp() supplies recorded times, and each pool connection sets synchronous_commit=on. The commit phase from Stage 1 runs in a spawned non-cancellable task: disconnect/deadline cannot undo publication; uncertain waiting yields outcome_unknown; clients resolve by idempotency key when no operation ID is known. Stage 0 stubs never begin mutations; a deadline returns dependency_unavailable with bounded retry guidance. Control-plane transaction behavior is verified by `control_plane` CP4, CP5 and CP13; file publication remains later work.

## Credential epochs and sessions

Credential rotation/revocation increments that credential's epoch; role/permission-policy changes increment workspace authorization_epoch. Capture both at operation start and recheck at publication. admin_operations stores credential_epoch_at_start; operations additionally stores workspace and session epochs. New read/handle operations check current authority. Issuer revocation cascades to issued credentials and their sessions. Read/write/admin roles retain server permission checks; only owner or workspace administrator may chmod, ownership changes require administrator. Workers cannot issue replacement sessions or acquire leases. Credential/session behavior is verified by `control_plane` CP7–CP12; role changes, filesystem permissions and leases remain later work.

Receipt keys are workspace + authenticated credential + idempotency key, with digest binding and 24-hour retention; deployment/session-minting routes use admin_operations. An expired ambiguous operation returns operation_indeterminate; it must not be automatically replayed under a new key/session. max_retry_attempts bounds retries. Client sessions are bounded and receipt replay cannot grant a new mutation. Control-plane receipts are verified by CP4, CP5 and CP12.

## Exclusive workflows

The workspaces authority row stores workflow owner/token hash/expiry. Acquire, release and expired takeover advance mutation_epoch under FOR UPDATE and fence old uploads; renew serializes there too. All mutations require the active token; readers may continue without snapshot semantics. A session tied to an expired lease fails closed until explicit host rebind. Workflow partial commits are not rolled back on command failure. Unverified until Stage 1/2.

## Object format and deployment identity

OBJECT_FORMAT_VERSION=1: an object is raw bytes of one chunk without framing. Its independent opaque key is deployment_id/workspace UUID/object UUID. SHA-256 lives in objects and manifest_chunks with a composite digest FK. The manifests/manifest_chunks tables define manifest format. Absent indexes and bytes beyond logical_length read as zeros; shrink/regrow must never expose old discarded tails. Manifest updates are rejected by schema triggers (verified: manifests_immutable); object persistence and read semantics remain unverified until Stage 1.

One deployment_id is both schema_meta identity and bucket prefix. Startup compares stored identity and configuration (verified: migrations_apply_revert_reapply). Two independent metadata authorities must never own the same prefix. Physical, staging and logical quota accounting and offline collection are Stage 1+ work; automatic deletion is disabled.

## Open deviations

TLS (U1): spec 03 requires TLS. Stage 0 listener is plaintext, restricted to loopback under the durable profile; development may bind all addresses. TLS_CERT_PATH/TLS_KEY_PATH are reserved and rejected. TLS termination is a Stage 1 decision. Verified boundary: serve_refuses_non_loopback_plaintext_durable.

Readiness lists one key under the deployment prefix through a retry-free client, with a bounded deadline and a one-second single-flight memo. It checks reachability, authentication, bucket existence and list permission; write permission, durability and replication remain unprobed. The service readiness setting `backend_probe_deadline` is deliberately outside the storage Timers table. TLS U1 and the development-only plaintext S3 rule are unchanged, so durable-profile end-to-end successful backend readiness remains unverified until U1. PostgreSQL reset_val plus role/database override checks detect the specified unsafe profiles; a privileged role setting can mask a lower-level server default, so these checks do not certify every deployment configuration or storage hardware.

## Linux/FUSE matrix

Official platform references consulted 2026-10-17. This is a target matrix, not a runtime compatibility claim.

| Target | Contract | Source | Status |
|---|---|---|---|
| Linux kernel | ≥5.10 with CONFIG_FUSE_FS | https://docs.kernel.org/filesystems/fuse/fuse.html | unverified until Stage 2 |
| FUSE protocol | ≥7.31 | https://github.com/torvalds/linux/blob/v5.10/include/uapi/linux/fuse.h | unverified until Stage 2 |
| mount helper | fusermount3, fuse 3.x | https://github.com/libfuse/libfuse | unverified until Stage 2 |
| Strict file profile | direct_io=true, writeback_cache=false, allow_mmap=false | spec 02 Reads and caching | profile serialization verified; kernel behavior unverified until Stage 2 |
| Metadata cache | attr_ttl=entry_ttl=negative_ttl=0 | spec 02 Reads and caching | profile serialization verified; kernel behavior unverified until Stage 2 |
| Permissions | default_permissions=true; server checks mandatory | spec 03 Identity and permissions | mount options compile; enforcement unverified until Stage 1/2 |
| Other OS | CLI prints profile; mount returns UnsupportedPlatform | independent plan D5 | macOS stub tests verified; other targets unverified |

Only default_permissions is a mount option here. direct_io is an open/reply flag and TTLs are reply fields to apply in Stage 2; no auto-cache/writeback option is enabled. mmap, flock/fcntl, ACLs, xattrs initially, device nodes, sockets/FIFOs and renameat2 flags beyond none are unsupported. No filesystem operation is implemented by the skeleton.

## Provider matrix

| Provider | Status |
|---|---|
| MinIO local | partially verified: `anchorfs-conformance/tests/backend.rs` F1–F15 against `bitnamilegacy/minio:2025.7.23-debian-12-r5@sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20`, single node, plaintext loopback. Delete excluded. Throttling, timeouts and lost responses are simulated by an in-test proxy. Checksum enforcement observed through one raw SDK request (F11b). No multi-node or durability claim. |
| DigitalOcean Spaces | unverified until Stage 3 live disposable contract suite |
| AWS S3 | later stage; unverified |

No object-store durability, FUSE runtime, provider compatibility or performance claim follows from Stage 0 builds, schema tests or SDK construction.

## Reviewed wire decisions

These are independent requirements/design clarifications from the Stage 0 code reviews; imported baselines remain unchanged. Future success responses are declared per operation, and Stage 0 initially returned 501 after shape validation. Stage 1a implements the thirteen control-plane routes listed below. Grant expiry strings are RFC3339 UTC timestamps. MutationResult carries operation_id and, when relevant, the affected inode/generation; WriteResult requires offset, bytes_written and committed generation (including the original offset on append retry). OperationStatus.result contains the original success metadata while retained, with secret/token fields omitted; an expired tombstone has state=indeterminate and result=null. Credential secrets and newly issued workflow tokens are returned by the initial grant response, never stored as plaintext in a receipt or returned by inspection. Successful workflow renewal may echo its currently supplied token without rotating it. A retry that would require replaying discarded grant material returns operation_indeterminate plus the operation ID; inspection still proves the committed metadata. A lost issued-credential secret is recovered by authorized rotation after inspection, and a lost workflow token requires waiting for expiry and authorized takeover (up to workflow_lease_ttl); neither case repeats the original mutation. Stage 1 must test these recovery paths before implementing grants.

For stat/list, an omitted query path selects the workspace root; for setAttributes an omitted JSON path selects root. Any supplied path remains nonempty and relative: empty, absolute, dot/traversal and invalid UTF-8 paths are rejected. Creation/removal targets still require a child path.

All workspace operations except getWorkspace, createSession and receipt inspection require X-Anchorfs-Session-Id (UUID) and X-Anchorfs-Session-Epoch (positive integer). This names the actor session; a session ID in the URL names the target session and does not replace actor authority. Authorized hosts first mint an ordinary actor session to acquire/take over a lease, then mint/rebind worker sessions to its grant. A bearer credential alone cannot select one of its multiple sessions. Deployment operations and session creation use admin_operations and require no preexisting actor session. Receipt inspection uses current authenticated original-credential scope, not an expired session.

Workflow authority is the paired X-Anchorfs-Workflow-Token (64 hex characters encoding 32 random bytes) and X-Anchorfs-Workflow-Epoch (positive mutation epoch). Both must be present or both absent. Renew/release require them. During an active lease, every mutation must present its current grant; ordinary operations may omit it only when no active lease applies. Bound sessions also record the grant epoch and fail closed after expiry until authorized host rebind. Stage 0 validates header shape only; Stage 1 must verify token hash, original credential, actor/target binding, current epochs/expiry and workflow/session authority under transaction locks.

GET /v1/admin-operations/{operation_id} and GET /v1/admin-operations?idempotency_key=... inspect original-credential-scoped administrative receipts. After a lost createWorkspace response, the authenticated original issuer can look up its original key without knowing either generated workspace ID or operation ID. It must not create again under a new key. Expired results return the retained tombstone through inspection and operation_indeterminate on mutation retry. B11 in persistent-state.md defines permanent identity/key retention and atomic session fencing; session renewal never revives a revoked epoch.

## Backend integrity contract

PUT verifies the local SHA-256 before uploading a nonempty object of at most the default chunk size. The default `checksum_mode=sha256` sends a signed SHA-256 checksum header: a matching provider echo confirms it, an absent echo triggers verified read-back, and a wrong echo is an integrity error. Mode `none` relies on transport integrity for successful uploads. SDK response checksum validation is disabled; AnchorFS verifies digest and length in `get_verified`. `get_range` verifies Content-Range and Content-Length framing only, and surfaces absent or clamped ranges rather than zero-filling. Both read forms share one configured request deadline across response headers and complete body consumption, including verified GETs used by upload read-back.

Ambiguous uploads reconcile through HEAD and, when needed, verified GET, repeating the same immutable bytes only after observed absence. The bound is two PUT, two HEAD and two GET calls, each with bounded SDK retries. `Unavailable` after reconciliation means not observed stored after the repeat and final HEAD; a delayed request may still land. Failed observation yields `UncertainOutcome`. Control-plane metadata publication is implemented; file publication and operations remain unimplemented. No Spaces/AWS compatibility or performance claim follows from local provider tests.

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
| `createCredential` kind=host | allowed (`workspace_id` required, else 400) | 403 | 403 |
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
409 operation_indeterminate with the operation ID. Session revocation at receipt
expiry is deferred by the dated B11 amendment in persistent-state.md. Find-by-key
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
