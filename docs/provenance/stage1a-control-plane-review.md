# Stage 1a implementation record

Implementation in progress on `feat/stage1a-control-plane`, starting from
`ef2c91ef1dbbc07abb7e86f9957b54284091c1c3`. This record does not assert completion
or review approval.

Approved inputs: the seven imported `docs/spec/` files, `docs/cleanroom.md`,
`docs/product.md`, existing AnchorFS implementation/tests/design records, explicit
user requirements, and the reviewed local `.agents/plans/stage1a-control-plane/`
plan. New code and synthetic tests are independently authored from those inputs.
No competitor implementation or research archive was accessed. This is a
procedural provenance statement, not legal clearance or proof of historical
isolation.

## Foundation

The core bearer grammar and hash comparison, persistence authority checks and
receipt transactions implement plan decisions D2, D4–D8 and D14–D22. Receipts
use the existing tables; migration 0007 adds only the two planned indexes.
SQL fragments interpolate only private enum-selected table names and fixed lock
clauses. Request values use bind parameters. SQLx 0.9 requires an explicit
`AssertSqlSafe` audit marker for these constructed statements.

No new package entered the dependency graph. SQLx's existing MIT/Apache-2.0
source is used with its `json` feature, persistence adds the already locked
serde_json dependency (MIT/Apache-2.0), and the service now directly names the
already locked SQLx dependency for transaction types and database error mapping.
All retain the sources and licenses recorded in `dependencies.md`.

Observed foundation verification:

- `cargo test -p anchorfs-core -p anchorfs-persistence --locked -- --test-threads=4`:
  8 core tests and 17 persistence integration tests passed in disposable PostgreSQL.
- Receipt contention observed a blocked second inserter, replay after owner
  commit, fresh ownership after owner rollback, and 55P03/aborted transaction on
  timeout. This is executable evidence for D18.
- Service tests exercise recursive canonical JSON, detached job completion,
  retained semaphore permits and unknown outcomes for panicking jobs.

Further verification and fresh review verdicts will be appended as completed.

## Workspace and credential package

Workspace creation, bootstrap, issuance, rotation, cascade revocation and
administrator receipt inspection implement the WP2 authority matrix. Secrets
are omitted from persisted results (`null` on replay), and fresh secret responses
carry `Cache-Control: no-store`. Bootstrap shares the metadata initialization
sequence with the server and reads only metadata configuration.

Observed verification: seven `control_plane_tx` integration tests passed,
including the issuer-lock/concurrent-child schedule, a cascade containing its
own authenticated actor, host lifetime/allowance bounds, and exactly one 40P01
victim for mutually rotating actors followed by successful retry with the new
proof. `bootstrap_admin_once` passed against disposable PostgreSQL. All five
OpenAPI snapshot tests and workspace all-target Clippy passed.

The mutual-rotation test makes an implicit plan detail explicit: the winning
rotation changes the deadlock victim's bearer secret, so its retry must use the
newly issued proof; retrying a stale bearer correctly returns unauthenticated.
The test does not bypass authentication for its retry.

## Session package

WP3 implements bounded minting, actor-session verification, renew/rebind/revoke
and workspace receipt inspection. The actor session uses an update lock when
it is also the target. The mint advisory lock follows receipt acquisition;
rotation revokes sessions so epoch-fenced sessions do not consume active slots.

Twelve persistence control-plane tests passed, including 63 existing sessions
plus two concurrent minters (exactly one success, one limit), lazy session
expiry, rotation freeing all slots, rebind clearing its binding and fencing old
epochs, own-session same-key renew replay without deadlock, and cross-session
renew producing exactly one 40P01 followed by successful retry. All five
OpenAPI snapshot tests passed after regeneration.

## HTTP conformance and records

CP2–CP13 execute through the real Axum router and metadata transactions in
separate disposable PostgreSQL containers. The counting stub implements all six
backend methods; zero calls are asserted for each fixture. This is a regression
guard, not network/process isolation evidence. Readiness keeps the existing MinIO
suite. The HTTP suite covers secret-free receipt replay, authority/scope failures,
64-session HTTP limits, rotation revoking all 64 sessions, self-fencing exceptions,
lazy tombstone writes, concurrent same-key smoke and the pending-owner budget.
CP13a observes publication after response abandonment. CP13b installs a test-only
deferred trigger in its disposable database, observes a real COMMIT waiting at an
advisory barrier through pg_stat_activity, terminates that connection, and checks
outcome_unknown plus matching state/receipt counts. No production fault hook or
trigger is introduced.

The contract test now explicitly names the thirteen implemented operation IDs
and asserts 13 authenticated routes plus 25 stubs. Body/header shape regressions
send a well-formed unknown bearer so they exercise validation before database
authentication. The fake backend and client helpers live in the planned common
modules; each HTTP scenario gets its own container for isolation rather than
sharing mutable administrator state.

Observed pre-review gates:

- `cargo test --workspace --locked -- --test-threads=4`: passed all 116 tests
  present at invocation, including 10 control-plane HTTP tests, 12 transaction
  tests, 17 schema tests, 9 binary tests, and 5 OpenAPI tests. No ignored tests.
- The additional sibling-cascade deadlock test passed separately. It observes
  exactly one 40P01. If the cascade loses, a current administrator retries
  successfully; if it wins, its revoked sibling actor correctly fails
  authentication on retry. This corrects the plan's unconditional “successful
  retry” wording without relaxing the authority matrix.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo deny check licenses bans advisories sources`: passed all four gates;
  duplicate-version warnings remain from the existing dependency graph.
- Toolchain: `1.95.0-aarch64-apple-darwin`; Docker server `29.2.1`.
- PostgreSQL and MinIO image digests remain the reviewed pins in dependencies.md.
- Design coverage still checks exactly 152 columns; workspace-member dependency
  edges and original imported baselines remain unchanged.

The CLI recovery check and direct tests of all four retryable SQLSTATE mappings
were added after that full-suite invocation and are verified in their targeted
runs and the subsequent review checkpoint gates.

## Limits retained

TLS remains unimplemented (durable HTTP binds loopback). Namespace, bytes,
workflow leases, Linux mounts, byte quota accounting and production provider
acceptance remain later milestones. Expiry is lazy, with no sweeper; receipt
expiry does not revoke/increment sessions until the byte-path lock-order decision.
A lost self-rotation response requires administrator/issuer recovery. A lost sole
unrevoked administrator secret requires explicit database maintenance before
bootstrap. Administrators may revoke themselves and every administrator; bootstrap
recovers when none remain. Detached jobs keep their semaphore permits but may
hold a connection beyond the response budget for ordinary lock waits. No rate
limiting is implemented. Cross-actor deadlocks surface retryable errors under
current authority; self-fencing replays can instead fail authentication/fencing.
No competitor parity, provider compatibility or storage durability claim follows
from these control-plane tests.

Fresh review verdicts and remote CI evidence remain pending at this checkpoint.


## Standard review findings and manual disposition

Exact review directory:
`reviews/feat-stage1a-control-plane-2026-10-18-150719-6dbd87933512`.
Transaction Correctness and Authority Architecture independently reviewed fresh
approved packets and both returned REQUEST_CHANGES with no blocking provenance
findings, 0 Critical / 2 Important / 0 Suggestions. Each independently reproduced
expired rebind exceeding capacity and publication after authority expiry; the
transaction reviewer also reproduced target-credential expiry during mint admission.
No prior reviewer conclusions or excluded research were supplied. The requested
opus model is unavailable; available reviewer agents were used, an explicit
procedural limitation rather than a claim that the model requirement was met.

Canonical fix-review --auto deduplicated the findings into two needs-manual items
because they affect authentication. It made no automatic edits and stopped the
checkpoint before commit/push. The implementer then applied a separate manually
inspected fix-review pass under the user's existing implementation authorization:

- TC1/AA2: mint and expired-session reactivation now share admission serialization;
  full capacity rejects reactivation, active rebind retains its slot. Concurrent
  create/rebind and rebind/rebind cases enforce one activation into the last slot.
- TC2/AA1: a database-clock guard checks captured credential/actor-session expiry
  after waits at publication, including receipt replay/inspection. Original
  lifetimes preserve intentional self-fencing semantics. Mint checks target expiry
  after admission and insertion. Business errors roll back fresh state and receipt.

These fixes enforce the approved session bound and spec 02 publication predicate;
no scope narrowing or imported specification amendment was used. Local contract
records clarify admission and actual target-lock ordering. Reviewers' follow-ups
are labeled rechecks, not fresh independent initial reviews.

New regression evidence: 16 persistence control-plane tests passed, including
all three new expiry/admission tests. HTTP regressions observe real database lock
waits before expiry, then prove 401/409 plus unchanged state and no fresh receipt.
A 64-active-plus-one-expired HTTP case checks 507 and active rebind at capacity.
Reviewer rechecks and resumed checkpoint gates remain pending.


The first recheck confirmed a residual TC1/AA2 schedule: a renewal's uncommitted
extension can be hidden behind the old committed expiry while create counts
slots. Both reviewers independently identified the gap; the transaction reviewer
reproduced it using a test-only disposable-database barrier. Renew now takes the
same admission lock through COMMIT and evaluates target expiry after acquiring it.
`renewed_session_does_not_release_a_hidden_mvcc_slot` holds a real renewal
uncommitted past the old expiry and observes the minter block, then reject at
capacity after renewal commits. This is the same bounded-session requirement,
not a scope change. Rechecks continue on this additional manual fix.


Final standard rechecks: Transaction Correctness and Authority Architecture both
returned APPROVE with no blocking provenance findings on the manually inspected
fixes, including renewal's MVCC interval. The former independently reran 17
transaction and 14 HTTP tests plus its repaired scratch reproduction; the latter
ran 17 transaction, 14 HTTP, 3 design and 3 provenance tests. Both rechecks are
same-context continuations. Authority Architecture's first recheck was interrupted
by a tool content filter; it resumed ordinary code inspection/repository tests
and completed, so no review stage is inferred from the interrupted attempt.

Canonical auto validation was resumed against the same manifest/head: both
findings are already resolved; no automatic auth edits, no outstanding
needs-manual item. Original findings and both structured summaries remain in the
exact review directory. The first checkpoint now runs the discovered CI gates
before its dedicated commit and push. No task-created PostgreSQL/MinIO container
remains; a pre-existing `web-api-postgres-1` Compose container was identified by
creation/Compose metadata and left untouched.

Resumed standard checkpoint gates passed: format, all-target Clippy, workspace
build, all 125 workspace tests, dependency licenses/bans/advisories/sources,
and diff whitespace. The Docker-free core/backend/mount subset and workspace
test compilation also passed. Remote CI remains pending PR creation.


## Thermonuclear structural pass

Pinned source:
https://raw.githubusercontent.com/cursor/plugins/0b8e952052d482ee6de06deebf14d590d77b1b34/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md
(commit `0b8e952052d482ee6de06deebf14d590d77b1b34`). It was fetched from the live
main identity after the first verified push, its frontmatter was validated, and
the entire rubric applied as criteria only. No commands/edits/scope authority was
granted by the external text.

Fresh reviewer directory:
`reviews/feat-stage1a-control-plane-thermo-2026-10-18-152402-781542bdad4c`.
The approved packet withheld this record's prior reviewer-history section from
the fresh reviewer under AGENTS.md; all changed implementation/contract/generated
files were read fully. This explicit exception preserves independent initial
judgment rather than silently feeding earlier conclusions. The reviewer returned
REQUEST_CHANGES for 3 Important structural items and 1 Suggestion, separately
reporting no blocking provenance findings. Independent evidence included 57
passing tests, offline license/source gates and unchanged baseline identities.

Canonical auto mode flagged all four as needs-manual because the affected
production and acceptance files concern credentials/authority. It made no edits
and stopped before commit/push. The separate manual fix-review pass addresses:

- T1: runtime DTO construction and explicit view conversion; typed secret grants
  select cache policy and preserve secret-null receipts. A new HTTP contract
  scenario deserializes fresh/replay/inspection responses into declared DTOs.
- T2: one receipt lifecycle helper owns begin/dispatch, final admitted-time check,
  fresh stored result and response separation. Callbacks keep route-specific
  authorization/locks/business operations explicit; direct persistence caller
  responsibilities are documented. No macro or trait hierarchy was introduced.
- T3: focused HTTP and transaction child modules plus shared support retain the
  original integration target names. All 31 original test bodies were moved
  byte-for-byte (hashes recorded in the local move manifest), and every new source
  file remains below 1,000 lines. The DTO scenario adds one test.
- S1: a receipt lookup enum removes invalid paired-option combinations and nullable
  lookup predicates, preserving by-ID/by-key visibility rules.

All code/tests are independently authored from existing DTOs, APIs, specifications
and the rubric's structural criteria. No dependencies or imported baselines
changed. Recheck and second checkpoint verification remain pending.


Thermonuclear recheck completed: APPROVE, no blocking provenance findings,
T1/T2/T3/S1 resolved. The reviewer independently ran 44 tests, formatting and
offline license/source checks, read all new modules, and compared all 31 original
scenario bodies directly against the checkpoint HEAD. This is a same-context
recheck; the original fresh verdict remains preserved. The approved packet's
review-history withholding exception remains explicit. Resumed canonical auto
validation reports all four already resolved and zero needs-manual items.
The second checkpoint now runs its discovered CI gates before commit/push.

Second checkpoint gates passed: format, all-target Clippy, workspace build,
all 126 workspace tests, full dependency license/ban/advisory/source checks,
and diff whitespace. All original backend, readiness, schema and binary tests
remain in that passing full run. The generated OpenAPI contract is unchanged by
the structural pass. Remote CI and actual PR evidence follow publication.
