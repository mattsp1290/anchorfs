# Stage 2a workflow leases evidence

## Inputs and origin

Implementation starts from `c432fa4d6e94262b6e1ebf6b388170783ea78745` on
`feat/stage2a-workflow-leases`. Approved inputs: the reviewed Stage 2a plan,
`docs/spec/`, repository code/tests/design and explicit user requirements.
All seven imported spec hashes were recomputed against import-manifest.json
and match. No competitor material, research archive, SDK, trace or private
protocol was consulted. New Rust/SQL, test schedules and fixtures are
independent designs from these requirements. No dependency, edge or migration
was added. The owner subsequently approved exactly two transitive security
patch updates; [D-WL1](stage2a-implementation-amendments.md) records their
source/license review and the lockfile-constraint exception. Imported baselines
remain unchanged.

## Local verification checkpoint

- At implementation commit `2513967cd1b139e382521ceaf1e8d4f10c32b9ca`, the full
  workspace test command completed with exit 0. This did not cover every
  concurrency schedule; subsequent corrections add explicit lock-order and
  abandoned-grant regression tests.
- `object_staging_receipt_fence_and_lease_have_no_fk_lock_cycle`: passed against
  pinned disposable PostgreSQL, using actual staging, fencing, receipt and lease
  helpers and checking terminal receipt/quota/orphan state.
- Namespace workflow suite after corrections: 10 passed, including the HTTP
  deadline regression and eight blocked acquire requests before holder release.
- Original persistence schedules: 15 passed; publication-load observation was
  113 ms in the recorded local run. No latency guarantee is inferred.
- Byte workflow suite: 3 passed against pinned disposable PostgreSQL/MinIO.
- Post-correction workspace/all-target clippy with warnings denied: passed.
- `cargo deny check licenses bans advisories sources`: all four checks pass
  after the owner-approved two-package security exception (D-WL1).
- Post-patch backend/core/mount tests and all three provenance checks passed.
- Final full post-correction/post-patch workspace regression: running. Hosted CI
  and final independent review gates remain pending; no result is inferred.
- Push preflight previously verified repo root, origin SSH, feature branch,
  authenticated repo scope and ADMIN permission; it is repeated before pushing.

## Review and hosted gates

The canonical fixer applied all six selected diagnostic fixes under the
owner's explicit instruction to continue autonomously. A fresh review pair
will assess this corrected branch using an approved-input packet and no prior
reviewer conclusions. Earlier diagnostic artifacts are retained outside this
record and do not satisfy the independent gate. Hosted CI remains pending.
Fresh contexts are procedural separation, not legal clearance or erased
historical exposure.

## Limits

No Linux mount, Spaces/AWS, lease restart or production sandbox claim. Expiry
is lazy, with no sweeper. Bound reads survive mere expiry and fail after epoch
advance. Revoked-owner leases persist until expiry/takeover. Exclusive lock
waits use the response budget, but a small post-lock/commit abandonment window
can lose the token; recovery is expiry/takeover. Sustained shared traffic has
no proven acquisition bound. TLS U1 and object deletion remain later work.
