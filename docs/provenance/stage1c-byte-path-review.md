# Stage 1c byte path — implementation evidence (in progress)

This record is incremental. Stage 1c is **not complete**. All eight planned
byte/pin routes are connected, with 34 implemented routes and four workflow-lease
stubs verified by the service sweep. The native byte path is exercised against
disposable PostgreSQL/MinIO. The full acceptance matrix, model and process-restart
evidence, review gauntlet, PR and hosted CI remain incomplete; no provider storage
durability or Linux mount capability is claimed.

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

## Write pipeline checkpoint (2026-10-20)

Implemented WP2: detached data runner with an atomic admission handshake,
hard backend deadlines and bounded final-phase acquisition; receipt-first
writeAt/append; exact reservations and staged objects before PUT; conservative
failure finalization; publication-time epoch/handle/lease/reservation fences;
bounded generation retries and an atomic reservation swap; truncate and
receipt-free fsync. The route sweep now observes **30 implemented / 8 stubs**.
OpenAPI was regenerated for the four connected methods and fsync's attribute
response without an idempotency-key parameter.

Origin remains approved specs, plan W2/W5–W9/W11/W14–W22/W25–W30/W32/W35, and
independent AnchorFS code. New helpers put inode loading and snapshot quota
checks in persistence, with transaction orchestration in service. No new crate,
dependency edge, imported text, competitor source or compatibility claim was
introduced. The implementation adds an explicit lock-timeout argument to the
lazy quota-pressure fence; otherwise its standalone transactions could inherit
PostgreSQL's unbounded default lock timeout. Uploaded buffers are released as
PUT completes, retaining lengths/digests/identities for publication and cleanup
so retry preparation does not retain a previous attempt's file buffers.

Observed verification so far:

- Service unit tests: 15 passed, including the admission race, prevention of
  late admission after timeout, and completion/permit retention after admitted
  response abandonment.
- Conformance service contract suite: 17 passed, including 30/8 route coverage.
  Its finite-body helper now sends Content-Length, matching HTTP framing when
  asserting rejection above the body limit before bearer parsing.
- New real-backend byte tests pass individually: complete manifests and sparse
  truncate/regrow; digest/key binding and empty-body rejection; lost-response
  receipt replay after close; conservative upload failure charges; concurrent
  append retry with the staging budget exactly full; post-upload credential,
  epoch and handle fencing. A single dropped PUT was initially recovered by
  the backend's documented two-round immutable retry; the failure fixture was
  corrected to drop both rounds and the terminal-failure test passed.
- `cargo test --workspace --locked -- --test-threads=4`: exited 0, 230 passing
  tests. This broad run began before the last buffer-release and receipt-error
  annotation refinements; the entire 8-test byte suite was rerun after them
  and passed, including forced conflict/retry and finalization cases.
- Final `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  formatting and diff checks passed. `cargo deny check licenses bans advisories
  sources` also exited 0 (existing duplicate warnings). No imported-spec or
  Cargo.lock diff.

The byte tests currently observe committed contents independently through
manifest SQL and raw SDK GETs, without the service planners. Native readRange,
pins, streamed conditional replace and the rest of BY1–BY21/model/restart
coverage still remain. No PR or gauntlet completion is claimed.

## Reads, pins and streamed replacement checkpoint (2026-10-20)

WP3 connects native readRange, createReadPin, releaseReadPin and replaceFile.
Reads capture one manifest in a short transaction, then fetch and verify whole
objects outside any transaction before slicing valid logical prefixes and
zero-filling sparse extents. Pinned reads use the pin's immutable manifest and
data generation; unpinned reads use the inode generation captured with the
current manifest. Pin admission is session-bound, bounded and receipt-bearing
in read lease mode, including during an active fixture workflow lease.

Replace validates headers without consuming the body, resolves the receipt
first, reserves the declared length, stages each chunk before PUT, verifies the
complete length/digest and publishes conditionally in one final transaction.
All body polls and backend calls use the request's hard deadline. Empty replace
uses one atomic transaction and no reservation, and malformed empty bodies
retain a terminal failed receipt. No transaction spans body or backend I/O.

The reviewed-input record and formal engineering details are consolidated in
[implementation amendments](stage1c-implementation-amendments.md).
These include the durable B11 flag (D-C1), the additional already-locked
http-body-util 0.1.5 runtime edge for typed length-error classification (D-C2),
and the single-transaction empty path inside the data runner (D-C3). The typed
error dependency's exact source/license and provenance row were checked before
use; the consolidated plan amendment followed local testing, as explicitly
recorded. No package version or package set changes. The dependency and edge
provenance suite passed after both new runtime edges were added.

Observed validation so far:

- `cargo test -p anchorfs-conformance --test bytes --locked`: 17 passed before
  the final exact two-chunk/cross-session acceptance case was added. The suite
  proves native read/replace concurrency coherence, corruption outside a read's
  requested prefix and missing-object errors, pinned generation/isolation/limit/
  release/expiry behavior, pin admission during a fixture lease, first PUT before
  the replacement body ends, replay without polling the body, strict conditional
  conflict after chmod, malformed digest/length failure, zero-reservation empty
  replace, stream disconnect/deadline cleanup, and lost responses for both empty
  and nonempty replacements. Workspace/deployment/bucket oracles are checked.
- A length-overrun test initially received dependency_unavailable because the
  body middleware emits a typed length-limit error. The new typed classification
  corrects that to content_length_mismatch without conflating disconnects, and
  the entire byte suite passed afterward.
- `cargo test -p anchorfs-service --lib --locked`: 15 passed.
- `cargo test -p anchorfs-conformance --test service --locked`: 17 passed,
  verifying 34 implemented / 4 stubs and header/shape validation.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
  before the final cross-session test addition.
- `cargo test -p anchorfs-conformance --test provenance --locked`: 3 passed.
- `cargo deny check licenses bans advisories sources`: exited 0, existing
  duplicate-version warnings only.
- OpenAPI regenerated. The WP3 workspace regression run exited 0 with 240
  passing tests, including all 18 byte tests at that checkpoint. This preceded
  the additional WP4 tests described below.

Remaining work is the full WP4 acceptance matrix (including variants and fault
schedules not established by these focused tests), bytes_model, service process
restart, final records, both gauntlet stages, and actual PR/hosted CI evidence.


## WP4 model, recovery and concurrency checkpoint (2026-10-20)

New tests are independently authored from the approved plan, the imported
specifications and AnchorFS contracts. Payloads are synthetic; fault schedules
use the repository's injector/proxy, disposable PostgreSQL and MinIO. No external
research or implementation input was consulted.

Observed targeted validation:

- `serve_restart_preserves_bytes`: passed. The real binary accepts 6,000 bytes
  over HTTP, returns them through native reads, dies by SIGKILL, then restarts
  against the same disposable database and bucket. A new session and handle see
  identical bytes and the same generation. Readiness and both SQL invariant
  checkers pass. No container or storage crash durability claim is made.
- `bytes_model`: 32 generated sequences passed; measured model execution
  21.273 seconds (28.74 seconds including fixture setup). It maintains a
  `BTreeMap` of dense file contents and inode/data generations for three files,
  checking native reads, stat and SQL/bucket invariants after each operation.
  Operations include write, append, truncate, conditional replacement (including
  stale generation), chmod, pin/read/release, close/reopen, create/open and
  truncate-on-open. The first run's oracle incorrectly treated truncate-on-open
  of an empty file as a generation no-op; the documented open contract always
  publishes a new empty manifest, and the corrected oracle passed. A final
  strengthening adds random unpinned offsets and a publication between pin
  creation and its snapshot read; all 32 sequences passed in 27.512 seconds
  (35.06 seconds including setup). Concurrent publication is tested separately
  by the BY schedules.
- `bytes recovery`: four tests passed. Pending same-key requests return the
  owner's operation id and later replay its result. Prepublication crashes leave
  staged objects and active charges; same-key and new-key/quota-pressure paths
  both durably fence expired reservations, persist failed receipts and charge
  candidates. Sub-chunk quota tests observe exact reservations of 900, 950 and
  1,000 bytes and reject oversized materializations before upload. Three 1,500
  byte appends and a subsequent far write/append follow current EOF exactly.
- `bytes concurrency`: five tests passed. Cross-session overlapping writes retry
  without losing unrelated bytes; a one-attempt configuration terminates with
  generation_conflict; chmod alone causes no byte retry; truncate forces a
  retry whose old tail becomes zeros. Two lost append responses replay distinct
  offsets with exactly-once contents. Five pipeline wait points and delayed GET
  and PUT each show zero idle-in-transaction connections. Different-file writes
  from two sessions completed in 117.961 ms wall time in this local run; this is
  an observation, not a throughput or latency SLO.
- `bytes quotas`: two tests passed. Physical, staging, principal and early
  logical limits reject without PUT; a late logical quota race fails publication
  and retains uploaded orphan charges. Workspace, deployment and bucket
  invariants pass.

The full workspace run including the 29 byte tests, initial model and real
binary restart exited 0: 253 tests passed. The strengthened model and three
additional access tests passed in targeted runs after that run began. Final
formatting, clippy with warnings denied and diff checks pass. Cargo deny passes
all four categories (existing duplicate warnings); all seven imported hashes
match the original manifest, and the diff against main for imported baselines
is empty. The remaining
WP4 matrix includes additional expiry/auth/lease/permission/provider variants,
COMMIT connection loss, restart over pending state, deployment contention,
randomized concurrent histories and pool saturation. Both gauntlet stages,
PR creation and hosted CI evidence remain outstanding.


The additional `bytes access` suite passed three tests: read/write handle access,
replacement of a mode-000 file by an explicitly issued admin-role host,
directory/symlink/read-role replacement rejection without receipt admission,
real one-second pin TTL, and expired-handle fsync without any receipt creation.
The first admin test attempt incorrectly used the fixture's write-role host;
that caller was correctly denied. The corrected fixture issues a distinct admin
role and uid, proving the intended permission bypass rather than relying on file
ownership. No production permission change was needed.
