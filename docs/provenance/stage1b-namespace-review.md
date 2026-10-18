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
