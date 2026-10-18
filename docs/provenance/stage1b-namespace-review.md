# Stage 1b implementation evidence

Implementation branch: `feat/stage1b-namespace-core`, based on the clean local `main` at start. The reviewed plan is `.agents/plans/stage1b-namespace-core/` (N1–N21). Implementation is in progress; this record does not claim completed routes or reviews.

## Approved inputs and origin

Inputs read: repository AGENTS.md, docs/README.md, docs/product.md, docs/cleanroom.md, specs 02/03/05, the reviewed Stage 1b plan, existing AnchorFS source/tests and design records. New Rust, SQL and synthetic test fixtures are independently authored from those inputs. No competitor implementation or external research archive was accessed. No dependency has been added yet. The seven imported baselines and import manifest remain unchanged.

## Foundation

Added the single-uid/gid permission helper, validated path components, strict 32-byte hex decoding, workflow owner/bound-session fencing, namespace guard/resolution, ordered inode locks, logical quota helper, charge marker migration and persisted-state invariant checker. Admin-role hosts can now be issued by deployment administrators. Configuration exposes handle/cursor limits and timers with a fixed test-only default signing key, replaced by from_env in production.

Implementation clarification: NamespaceGuard uses AtomicBool with relaxed ordering rather than Cell, allowing shared guard references across Send service futures. Transaction ownership still serializes use; the atomic is only an at-most-once generation marker, not a database lock.

Verified foundation commands:

- `cargo test -p anchorfs-core --locked`: 11 passed.
- `cargo test -p anchorfs-persistence --locked --test namespace_tx`: eight foundation scenarios, including observed epoch-writer blocking, workflow predicates, admin-host issuance and symlink traversal.
- `cargo test -p anchorfs-conformance --locked --test design_coverage`: three passed; 153 columns match documentation.
- `cargo test -p anchorfs-conformance --locked --test control_plane credentials`: four passed.
- `cargo test -p anchorfs-persistence --locked --test migrations migrations_apply_revert_reapply`: passed (migration 0008 applies/reverts).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all --check`, `git diff --check`: passed after fixing lint/test compile errors.
- All seven specification SHA-256 identities equal import-manifest.json.

The API document version changes with the regenerated snapshot in WP5 rather than changing the version while route handlers still return 501. No review stage has run. Deployment, provider, mount and byte-path claims are outside this milestone.

## Mutation implementation clarification

The literal sibling schedule `x → y/x`, then `y → x/y`, returns `not_found` for the second move because the namespace guard is acquired before path resolution and `x` is no longer a root child. The plan's expected `invalid_input` for that second request is inconsistent with its lock/walk order. The test requires exactly one success, observed namespace-lock blocking and an acyclic final tree; direct attempts at `a → a/x` and `a → a/b/c` still return `invalid_input`. This is a local test-expectation correction based on executable path semantics, not a change to the imported baseline.

## Namespace mutations (WP2)

Seven mutation routes now execute persistence business functions under the namespace guard and workspace receipts. `unlink` uses metadata deadlines. Parsed request DTOs are serialized for receipt digests. Nine persistence mutation tests passed, including observed lock waits for same-name create and sibling moves, directory replacement parent-link accounting, no-op rename and symlink charges. HTTP mutation coverage exercises all seven routes, identical replay, mismatched replay and receipt inspection with invariant checks and no backend calls. Service contract suite: 17 passed (20 implemented routes / 18 stubs at this checkpoint); service unit suite: seven passed; OpenAPI snapshot suite: five passed. Clippy, formatting and diff checks passed. Open-file retention scenarios are pending WP4; broader permission/recovery/model coverage remains WP5.

## Reads and cursors (WP3)

`statPath`, `listDirectory` and `readSymlink` return committed metadata with non-snapshot walks and no namespace lock. Keyset pagination uses C byte ordering, reports namespace_generation and authenticates scope/expiry using RustCrypto HMAC-SHA256 before parsing canonical JSON. Four cursor unit tests passed, including every-byte tampering and RFC 4231 case 2. HTTP read coverage passed for paging, C ordering, parent scope, malformed MAC, symlink reads and intermediate-symlink rejection. Service split is 23 implemented / 15 stubs; the changed wire-binding expectation passed after correction. Five OpenAPI snapshot tests passed. Clippy, formatting and diff checks passed. `cargo deny check licenses bans advisories sources` passed with existing duplicate-version warnings. Hmac direct-use metadata/license checked; the lock change adds only dependency edges. Full large-page and expiry HTTP coverage remains WP5.

## Metadata-only handles (WP4)

Open/create/exclusive/truncate-empty, renew and close now publish workspace receipts. Open requires read/write flags, appropriate role/mode permissions and a namespace guard for create. Renew locks inode → handle → admission; close locks inode → handle and uses the common quota-release predicate. HandlePolicy groups admission limit and TTL without changing the planned behavior. Migration 0008 landed in WP1.

Ten PostgreSQL handle tests passed: replacement/unlink retention, last-handle release, expiry, other-session invisibility, two concurrent closes, close/unlink interleaving, concurrent admission observed on an advisory lock, the three-actor renew/close/create schedule, expiry-boundary unlink and resolve/unlink locking. Two HTTP handle scenarios passed, including rename-over-open, unlink-open, stale close, receipt replay after close, and truncate-only inode generation. The wire-binding test passed at the final 26 implemented / 12 stub split; clippy passed.

Expiry schedule clarification: renewal must validate and extend before the old expiry, then park while holding the inode until unlink has passed the old expiry. Parking before update until after expiry would correctly produce stale_handle and cannot demonstrate a successful renewal. The implemented observed-wait schedule tests the intended serialization without resurrecting expired handles. Neither this schedule nor sequential model testing proves byte-path concurrency.
