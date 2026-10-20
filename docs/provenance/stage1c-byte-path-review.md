# Stage 1c byte path — implementation evidence (in progress)

This record is incremental. Stage 1c is **not complete**; byte and pin HTTP
routes still return their Stage 1b stubs. No end-to-end durable byte capability,
review-gauntlet completion, PR, hosted CI result or storage durability is claimed.

## Approved inputs and origin

Implementation began on 2026-10-20 on `feat/stage1c-byte-path`, from
`4cc0069836783ab032b911d49c07ae50bd26b32d`. The worktree was clean. The seven
`docs/spec/*.md` SHA-256 values were recomputed and matched
`docs/provenance/import-manifest.json`. Neither imported specifications nor
the manifest was changed.

Inputs consumed: repository AGENTS guidance, `docs/README.md`, `docs/product.md`,
`docs/cleanroom.md`, approved storage/interface requirements, the Stage 1c local
plan, and AnchorFS implementation/tests/design records. New Rust, SQL and
synthetic fixtures are independently authored from these inputs. No competitor
implementation, external research archive, private trace or copied fixture was
used. This is an origin statement, not independent review or legal clearance.

The requested dotfiles `review-gauntlet/SKILL.md` has been read. Its two ordered
review/fix/push stages remain pending until implementation and validation are
complete. Independent reviewers must receive fresh approved-input packets and
report the cleanroom checklist verdict separately from technical findings.

No dependency was added or updated in this checkpoint. All direct dependencies
remain the reviewed workspace pins; `Cargo.lock` is unchanged. The planned
service `futures-core` dependency edge has not yet been introduced.

## Implemented foundation so far

- Pure chunk write/read/truncate/replace planners and materialization, mapped
  to spec 02 components, truncate and read requirements and plan W2/W8/W9/W19.
  Trimmed logical prefixes preserve whole-object length and never copy old tails.
- Migration 0009 adds immutable manifest generation, deployment chunk size and
  reservation/object/pin indexes. Bootstrap and serve both validate and retain
  the configured chunk size (W19).
- Six byte-path configuration fields with principal concurrency, chunk bounds,
  reservation deadline/retention and positive pin/retry validations (W18/W25).
- Reservation, object, manifest and pin persistence primitives plus workspace
  and deployment staging/physical settlement (W6/W7/W9/W15/W34). These primitives
  are tested individually; receipt ownership, lazy fences, final publication
  authority, route orchestration and the full accounting oracle remain pending.
- Production AppState construction installs NoFaults and a bounded data
  semaphore. Fault points are inert until the pipeline is connected.

At checkpoint 933ac5e, WP1 was incomplete. Remaining work then was: receipt-owner and pending/failed
semantics, expired-receipt session revocation, overdue-reservation fences,
manifest-bearing file creation and truncate-on-open, handle read/write helpers,
accounting invariant extensions, byte-capable conformance fixtures, remaining
transaction schedules and design lock-order amendment. WP2/WP3 and the full
BY1–BY21/model/restart verification remain pending. The chunk arithmetic
property oracle was added early from WP4.

The receipt work exposed a schema detail (resolved below as D-C1): W35 excludes expired `failed` rows
from session revocation, including later encounters, while tombstones clear
`result`. The implementation must retain enough durable origin information to
distinguish those rows from tombstones of `pending`/`committed` operations after
a crash. The plan's two-column migration alone does not encode that distinction;
D-C1 below records the implemented resolution for the B11 follow-ups.

## Observed local validation

- `cargo test -p anchorfs-core --locked`: 11 existing unit tests and 12 new
  chunk planner tests passed.
- `cargo test -p anchorfs-persistence --test migrations --locked`: 17 passed,
  including apply/revert/reapply of migration 0009.
- `cargo test -p anchorfs-persistence --test bytes_tx --locked`: 7 passed,
  covering admission bounds, orphan charging, staging expiry/finalization,
  settlement, manifest copy/clip and pin bounds/ownership. One initial pin
  fixture incorrectly bootstrapped a second administrator; it was corrected
  to create a second session through the existing host authority and rerun.
- `cargo test -p anchorfs-service --test binary --locked`: 11 passed, including
  configuration bounds and bootstrap/serve/rebootstrap/chunk-size refusal.
- `cargo test -p anchorfs-conformance --test design_coverage --locked`: 3 passed;
  all 155 columns documented.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
  after the final constructor/property/manifest-test additions.
- `cargo deny check licenses bans advisories sources`: passed, with existing
  duplicate-version warnings. No lockfile change.
- Formatting and `git diff --check`: passed.

Only disposable pinned PostgreSQL and MinIO containers were used. There is no
Spaces/AWS, Linux mount, provider durability or consumer integration evidence.
`cargo test --workspace --locked -- --test-threads=4` exited 0, including
all Stage 0–1b suites and the new foundation tests. That invocation began before
the last constructor-test/property-test and replace assertion additions; those
were covered separately by passing `cargo test -p anchorfs-service --lib
--locked` (12 tests), `cargo test -p anchorfs-conformance --test chunking_props
--locked` (256 generated cases), and the rerun of `bytes_tx` (7 tests).
The imported-spec diff against main is empty.

## Receipt, manifest and fixture checkpoint (2026-10-20)

The next implementation step adds owner-proof pending receipts, conditional
terminal writers, read-only peek, lazy reservation expiry on retry/inspection
and quota pressure, and post-tombstone session follow-ups. The latter retry on
every encounter and retain the failed-outcome exclusion. New file creation
publishes an empty manifest; truncate-on-open publishes a new empty manifest
and settles logical bytes. Handle read/write grants and final lifetime checks
have persistence helpers. Workspace/deployment invariant checks now cover the
byte accounting and manifest rules.

The conformance fixture now supports real MinIO with an optional fault proxy,
binary requests/responses, a bounded pool, isolated deployment prefixes and a
container owned by a dedicated process-lifetime runtime. Bucket reconciliation
uses a raw SDK listing of the disposable workspace prefix, bypassing both the
service adapter and scripted proxy; it rejects unknown/uncharged keys and
missing published keys. This is a stronger independent observation boundary
than using only the service adapter's list path. Two fixture tests verify shared
MinIO isolation and that a planted uncharged object is detected. No byte HTTP
route has been connected by this checkpoint; the route split remains 26/12.

### Local schema detail D-C1

Requirement: W35 revokes sessions for tombstones of pending/committed receipts,
including encounters after process death, and excludes tombstones of failed
receipts. W35 also clears the stored result. The plan's proposed schema did not
retain which category produced an indeterminate tombstone. Independent local
implementation decision: migration 0009 additionally adds
`operations.expiry_fences_session boolean NOT NULL DEFAULT false`. Tombstoning
sets it from the prior locked state, and later reads/follow-ups check it.
Administrative receipts need no flag because they have no session.

This retains the original requested behavior rather than changing product
scope. There are no deployed users or compatibility requirements, per the
owner's plan decision. The persistent-column gate is consequently **156**,
rather than the plan's 155; the design table and executable coverage check were
updated together. No imported specification or import baseline was amended.
Origin: approved W35 plus independent crash-recovery/schema reasoning. Evidence:
`expired_receipt_tombstone_then_session_fence_once` tests pending, committed and
failed outcomes through multiple encounters and one-time epoch increment;
`expired_receipt_followup_recovers_after_a_crash` tests HTTP inspection, the
BeforeFollowUp fault, subsequent recovery, terminal rebind rejection and new
host-issued session recovery. Independent gauntlet review remains pending.

### Verification for this checkpoint

- `cargo test -p anchorfs-persistence --locked`: passed before the final new
  transaction tests, covering migration apply/revert/reapply and all regressions.
- `cargo test -p anchorfs-persistence --test bytes_tx --locked`: 18 passed after
  the receipt/handle/invariant additions. The race test observes a fence waiting
  on the publisher's operations row before commit; it then returns the original
  committed result and leaves physical/staging charges unchanged.
- `cargo test -p anchorfs-persistence --test namespace_tx --locked`: 30 passed.
- Conformance control-plane, namespace and design coverage: 16/18/3 passed
  before the additional crash test; the crash test passed separately. CP12 now
  expects the planned 409 fenced after the inspecting host revokes the worker's
  expired-receipt session, and proves host-issued recovery.
- `cargo test -p anchorfs-conformance --test bytes --locked`: 2 fixture tests
  passed against the real shared MinIO container.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo deny check licenses bans advisories sources`: passed (existing duplicate
  warnings). No new dependencies or lockfile changes.
- OpenAPI regenerated with `cargo run -p anchorfs-service --locked -- openapi
  --format yaml`; version 1.0.0-stage1c now includes operation_failed. Byte and
  pin route descriptions remain stubs until their implementation.
- `cargo test --workspace --locked -- --test-threads=4`: exited 0, 221 passing
  tests across the workspace and doc-test suites. This includes 26/12 route
  coverage, all namespace regressions, 156-column design coverage, OpenAPI
  snapshots, migration reversibility and the generated chunk model.
- After that run, the bucket negative control was tightened to require the
  specific bucket-uncharged panic, owner redaction checked on the whole receipt,
  and overlapping quota predicates checked for precedence. Both affected suites
  were rerun: bytes_tx 18 passed; byte fixture tests 2 passed. Final clippy,
  formatting and diff checks passed; imported hashes and Cargo.lock unchanged.

Still required: write/read/replace/pin pipelines and route implementations,
service-level fault, concurrency, quota, model and restart acceptance (BY1–BY21),
complete implementation/design records, both review-gauntlet stages and PR/CI.
