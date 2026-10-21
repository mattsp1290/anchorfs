# Stage 2b implementation evidence (in progress)

This record is incremental. Missing tests or reviews remain incomplete; the
plan is intent, not runtime evidence.

## Inputs and origin

Implementation began on `feat/stage2b-linux-mount` from
`1211158b6ee35e083e9785b1b181ceade4d441d6` on 2026-10-21. All seven imported
specification SHA-256 values and the overview's design, API, dependency,
lockfile and CI identities matched before editing. The imported baseline and
manifest remain unchanged.

New code and synthetic fixtures are independently authored from the approved
Stage 2b plan, `docs/spec/`, AnchorFS service DTOs/routes/tests, core path and
error types, and the chosen MIT-licensed fuser 0.18.0 dependency API. No
competitor implementation, client, SDK, private trace or research archive was
consulted. This statement describes inputs to this implementation session;
it is not legal clearance or a claim that fresh context erases past exposure.

`cargo info reqwest@0.12.28` and `cargo info libc@0.2.191` confirmed the
approved repository origins and MIT OR Apache-2.0 licenses before the new
edges. Cargo.lock currently adds only the client workspace member and workspace
edges; no registry package was added. The source of the native client wire
shapes is AnchorFS's own `dto.rs`, an approved repository input.

## Executed checkpoints (2026-10-21)

- Client unit tests: 2 passed (headers/plaintext policy; core errno hints).
- Client conformance: 3 passed against disposable PostgreSQL and MinIO.
  The response-loss case drops all three responses for each of mkdir, open,
  positional write, append and session creation; typed receipt lookup returns
  each original result, and SQL verifies one publication per mutation.
- Timestamp conformance: 2 passed (owner/admin/forbidden, invalid instants,
  symlink times, replay/digest mismatch, active lease gate).
- Timestamp persistence test: 1 passed (explicit instants, generation, empty
  update rejection).
- Service OpenAPI snapshot tests: 5 passed. Service binary tests: 12 passed,
  including process restart byte persistence.
- `cargo deny check licenses bans advisories sources`: passed (existing
  duplicate-version warnings).
- Linux host readiness: kernel 6.8.0-124-generic, non-root uid, /dev/fuse,
  fusermount3 3.14.0, Docker 29.2.1, local temp filesystem. No host mount test
  has run yet.

## Implementation corrections to the plan

Executable service routes and OpenAPI take precedence over the WP1 shorthand:
`unlink` and `remove_directory` send `path` as a query parameter, not a DELETE
JSON body. Conformance exercises both. Client retries derive the receipt scope
from the wrapped operation: session creation uses admin receipts; all other
wrapped mutations use workspace receipts.

## Remaining gates

Mount implementation, portable tests, real Linux kernel suite, Linux host
run, full regression gates, fresh independent cleanroom reviews, both review
gauntlet stages, hosted CI and PR creation remain incomplete.
