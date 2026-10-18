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
