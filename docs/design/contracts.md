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

Seconds. U2 defaults are proposed for owner acceptance at the Stage 0 review. All future operational timers must be configurable; Stage 0 only exposes the service deadlines.

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

READ COMMITTED, explicit locks and the complete order/modes in [persistent-state.md](persistent-state.md). PostgreSQL clock_timestamp() is the expiry authority, transaction_timestamp() supplies recorded times, and each pool connection sets synchronous_commit=on. The commit phase from Stage 1 runs in a spawned non-cancellable task: disconnect/deadline cannot undo publication; uncertain waiting yields outcome_unknown with operation ID. Stage 0 stubs never begin mutations; a deadline returns dependency_unavailable with bounded retry guidance. Locking/publication remain unverified until Stage 1.

## Credential epochs and sessions

Credential rotation/revocation increments that credential's epoch; role/permission-policy changes increment workspace authorization_epoch. Capture both at operation start and recheck at publication. admin_operations stores credential_epoch_at_start; operations additionally stores workspace and session epochs. New read/handle operations check current authority. Issuer revocation cascades to issued credentials and their sessions. Read/write/admin roles retain server permission checks; only owner or workspace administrator may chmod, ownership changes require administrator. Workers cannot issue replacement sessions or acquire leases. Unverified until Stage 1.

Receipt keys are workspace + authenticated credential + idempotency key, with digest binding and 24-hour retention; deployment/session-minting routes use admin_operations. An expired ambiguous operation returns operation_indeterminate; it must not be automatically replayed under a new key/session. max_retry_attempts bounds retries. Client sessions are bounded and receipt replay cannot grant a new mutation. Unverified until Stage 1.

## Exclusive workflows

The workspaces authority row stores workflow owner/token hash/expiry. Acquire, release and expired takeover advance mutation_epoch under FOR UPDATE and fence old uploads; renew serializes there too. All mutations require the active token; readers may continue without snapshot semantics. A session tied to an expired lease fails closed until explicit host rebind. Workflow partial commits are not rolled back on command failure. Unverified until Stage 1/2.

## Object format and deployment identity

OBJECT_FORMAT_VERSION=1: an object is raw bytes of one chunk without framing. Its independent opaque key is deployment_id/workspace UUID/object UUID. SHA-256 lives in objects and manifest_chunks with a composite digest FK. The manifests/manifest_chunks tables define manifest format. Absent indexes and bytes beyond logical_length read as zeros; shrink/regrow must never expose old discarded tails. Manifest updates are rejected by schema triggers (verified: manifests_immutable); object persistence and read semantics remain unverified until Stage 1.

One deployment_id is both schema_meta identity and bucket prefix. Startup compares stored identity and configuration (verified: migrations_apply_revert_reapply). Two independent metadata authorities must never own the same prefix. Physical, staging and logical quota accounting and offline collection are Stage 1+ work; automatic deletion is disabled.

## Open deviations

TLS (U1): spec 03 requires TLS. Stage 0 listener is plaintext, restricted to loopback under the durable profile; development may bind all addresses. TLS_CERT_PATH/TLS_KEY_PATH are reserved and rejected. TLS termination is a Stage 1 decision. Verified boundary: serve_refuses_non_loopback_plaintext_durable.

Readiness: spec 04 ultimately requires backend capability checks. Stage 0 reports backend=not_checked and checks only database/version/durability readiness. Backend checks remain unverified until the Stage 1 MinIO contract suite. PostgreSQL reset_val plus role/database override checks detect the specified unsafe profiles; a privileged role setting can mask a lower-level server default, so these checks do not certify every deployment configuration or storage hardware.

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
| MinIO local | unverified until Stage 1 contract suite |
| DigitalOcean Spaces | unverified until Stage 3 live disposable contract suite |
| AWS S3 | later stage; unverified |

No object-store durability, FUSE runtime, provider compatibility or performance claim follows from Stage 0 builds, schema tests or SDK construction.

## Reviewed wire decisions

These are independent requirements/design clarifications from the Stage 0 code reviews; imported baselines remain unchanged. Future success responses are declared per operation, but every Stage 0 operation still returns 501 after shape validation. Grant expiry strings are RFC3339 UTC timestamps. MutationResult carries operation_id and, when relevant, the affected inode/generation; WriteResult requires offset, bytes_written and committed generation (including the original offset on append retry). OperationStatus.result contains the original success metadata while retained, with secret/token fields omitted; an expired tombstone has state=indeterminate and result=null. Credential secrets and newly issued workflow tokens are returned by the initial grant response, never stored as plaintext in a receipt or returned by inspection. Successful workflow renewal may echo its currently supplied token without rotating it. A retry that would require replaying discarded grant material returns operation_indeterminate plus the operation ID; inspection still proves the committed metadata. A lost issued-credential secret is recovered by authorized rotation after inspection, and a lost workflow token requires waiting for expiry and authorized takeover (up to workflow_lease_ttl); neither case repeats the original mutation. Stage 1 must test these recovery paths before implementing grants.

For stat/list, an omitted query path selects the workspace root; for setAttributes an omitted JSON path selects root. Any supplied path remains nonempty and relative: empty, absolute, dot/traversal and invalid UTF-8 paths are rejected. Creation/removal targets still require a child path.

All workspace operations except getWorkspace, createSession and receipt inspection require X-Anchorfs-Session-Id (UUID) and X-Anchorfs-Session-Epoch (positive integer). This names the actor session; a session ID in the URL names the target session and does not replace actor authority. Authorized hosts first mint an ordinary actor session to acquire/take over a lease, then mint/rebind worker sessions to its grant. A bearer credential alone cannot select one of its multiple sessions. Deployment operations and session creation use admin_operations and require no preexisting actor session. Receipt inspection uses current authenticated original-credential scope, not an expired session.

Workflow authority is the paired X-Anchorfs-Workflow-Token (64 hex characters encoding 32 random bytes) and X-Anchorfs-Workflow-Epoch (positive mutation epoch). Both must be present or both absent. Renew/release require them. During an active lease, every mutation must present its current grant; ordinary operations may omit it only when no active lease applies. Bound sessions also record the grant epoch and fail closed after expiry until authorized host rebind. Stage 0 validates header shape only; Stage 1 must verify token hash, original credential, actor/target binding, current epochs/expiry and workflow/session authority under transaction locks.

GET /v1/admin-operations/{operation_id} and GET /v1/admin-operations?idempotency_key=... inspect original-credential-scoped administrative receipts. After a lost createWorkspace response, the authenticated original issuer can look up its original key without knowing either generated workspace ID or operation ID. It must not create again under a new key. Expired results return the retained tombstone through inspection and operation_indeterminate on mutation retry. B11 in persistent-state.md defines permanent identity/key retention and atomic session fencing; session renewal never revives a revoked epoch.
