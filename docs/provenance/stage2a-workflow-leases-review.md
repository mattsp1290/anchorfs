# Stage 2a workflow leases evidence

## Inputs and origin

Implementation starts from `c432fa4d6e94262b6e1ebf6b388170783ea78745` on
`feat/stage2a-workflow-leases`. Approved inputs: the reviewed Stage 2a plan,
`docs/spec/`, repository code/tests/design and explicit user requirements.
All seven imported spec hashes were recomputed against import-manifest.json
and match. No competitor material, research archive, SDK, trace or private
protocol was consulted. New Rust/SQL, test schedules and fixtures are
independent designs from these requirements. No dependency, edge or migration
was added; the lockfile and imported baselines remain unchanged.

## Local verification in progress

- Original persistence regression suite: passed.
- New persistence workflow schedules: 15 passed; publication-load observation
  was 113 ms in the recorded local run. No latency guarantee is inferred.
- Byte workflow suite: 3 passed against pinned disposable PostgreSQL/MinIO.
- Clippy workspace/all-targets/locked with warnings denied: passed.
- Full workspace regression: first run used a stale version of WL-N6 and
  reported its fixture failure; the corrected focused WL-N6 passes. The 52 byte
  regressions passed. A final full run is still required.
- `cargo deny check licenses bans advisories sources`: licenses/bans/sources
  pass; advisories fail for existing rustls-webpki 0.103.15
  (RUSTSEC-2026-0382) and webpki-roots 1.0.9 (RUSTSEC-2026-0381).
  Owner decision requested before changing the explicit unchanged-lockfile gate.
- Push preflight: repo root, origin SSH, feature branch, gh authentication with
  repo scope and ADMIN repository permission verified; PREFLIGHT_RESULT=READY.

## Review and hosted gates

Review gauntlet and hosted CI are pending. No review verdict is claimed yet.
Fresh reviewers must receive only the approved-input packet and branch evidence;
prior reviews/research history are excluded. Fresh contexts are procedural
separation, not legal clearance or erased historical exposure.

## Limits

No Linux mount, Spaces/AWS, lease restart or production sandbox claim. Expiry
is lazy, with no sweeper. Bound reads survive mere expiry and fail after epoch
advance. Revoked-owner leases persist until expiry/takeover. Exclusive lock
waits use the response budget, but a small post-lock/commit abandonment window
can lose the token; recovery is expiry/takeover. Sustained shared traffic has
no proven acquisition bound. TLS U1 and object deletion remain later work.
