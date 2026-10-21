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
- Full post-correction/post-patch workspace regression passed at the standard
  checkpoint. The thermonuclear checkpoint also passed fmt, all-target clippy with
  warnings denied, all four cargo-deny checks, the full workspace test suite,
  workspace build, workspace no-run compilation and diff checks. This includes
  52 byte, 26 namespace, 32 control-plane transaction, 19 byte transaction,
  20 service unit, 12 binary and five OpenAPI tests, both models and
  provenance/156-column coverage. Generated YAML matches byte-for-byte;
  imported hashes and unchanged migrations were rechecked. Hosted CI follows
  draft PR creation.
- Push preflight previously verified repo root, origin SSH, feature branch,
  authenticated repo scope and ADMIN permission; it is repeated before pushing.

## Review and hosted gates

The canonical fixer applied all six selected diagnostic fixes under the
owner's explicit instruction to continue autonomously. Earlier diagnostic
artifacts are retained but do not satisfy the independent gate.

The valid standard review used fresh contexts and an explicit approved-input
packet, with no peer conclusions shared. Lease Transactions and Contract
Evidence both returned APPROVE and no blocking provenance findings. The
standard fixer resolved one documentation suggestion (fixed:1, skipped:0,
needs-manual:0), then passed fmt, all-target clippy, cargo-deny, the full workspace
tests, build, no-run and diff checks. Checkpoint
`ffc70cef87bfeb2f2ea85a2389b81afa55332898` was pushed and verified against origin.
Artifacts: `reviews/feat-stage2a-workflow-leases-2026-10-20-213956-8224def2da74`.

After that verified push, the live upstream thermonuclear rubric was fetched
at `cursor/plugins` commit `190d28736eff128ec94f502a22c7fe0c4b56cd37` from
https://raw.githubusercontent.com/cursor/plugins/190d28736eff128ec94f502a22c7fe0c4b56cd37/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md .
Its complete quality criteria were applied by a fresh adversarial reviewer.
The initial verdict was REQUEST_CHANGES for two Important structural findings,
with no blocking provenance findings. The canonical fixer resolved both
(fixed:2, skipped:0, needs-manual:0): focused workflow test modules and one typed
rebind request owning digest/execution. The same reviewer's labeled recheck
returned APPROVE within the fix/regression scope, no new findings and no
blocking provenance findings; it independently passed 13 relevant tests and
verified all ten workflow test bodies are byte-identical. Artifacts:
`reviews/feat-stage2a-workflow-leases-thermonuclear-2026-10-20-215545-ffc70cef87bf`.

Both initial review stages read the entire diff and all full changed files.
The requested model alias was unavailable; the available inherited runtime was
used and disclosed. No particular unavailable model is claimed. Fresh contexts
are procedural separation, not legal clearance or erased historical exposure.
Thermonuclear checkpoint `5fefab1d19ecfacde8a97574ef444da7fe4a9352` was
pushed and verified against origin before opening draft
[PR #6](https://github.com/mattsp1290/anchorfs/pull/6). All five hosted jobs
(lint, deny, Linux workspace tests, Linux ARM64 build and macOS
build/clippy/test-compilation/core-backend-mount tests) passed in
[CI run 39531660970](https://github.com/mattsp1290/anchorfs/actions/runs/39531660970)
for that exact code revision. The final evidence-only update records this
completed run; it adds no code or dependency changes.

## Limits

No Linux mount, Spaces/AWS, lease restart or production sandbox claim. Expiry
is lazy, with no sweeper. Bound reads survive mere expiry and fail after epoch
advance. Revoked-owner leases persist until expiry/takeover. Exclusive lock
waits use the response budget, but a small post-lock/commit abandonment window
can lose the token; recovery is expiry/takeover. Sustained shared traffic has
no proven acquisition bound. TLS U1 and object deletion remain later work.
