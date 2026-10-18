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
