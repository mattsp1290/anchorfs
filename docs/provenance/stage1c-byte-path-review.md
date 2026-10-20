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

The full WP1 is not done. Still required: receipt-owner and pending/failed
semantics, expired-receipt session revocation, overdue-reservation fences,
manifest-bearing file creation and truncate-on-open, handle read/write helpers,
accounting invariant extensions, byte-capable conformance fixtures, remaining
transaction schedules and design lock-order amendment. WP2/WP3 and the full
BY1–BY21/model/restart verification remain pending. The chunk arithmetic
property oracle was added early from WP4.

Implementation note for the receipt work: W35 excludes expired `failed` rows
from session revocation, including later encounters, while tombstones clear
`result`. The implementation must retain enough durable origin information to
distinguish those rows from tombstones of `pending`/`committed` operations after
a crash. The plan's two-column migration alone does not encode that distinction;
resolve and record this local schema detail before implementing B11 follow-ups.

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
