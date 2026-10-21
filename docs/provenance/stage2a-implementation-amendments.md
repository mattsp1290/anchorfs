# Stage 2a implementation amendments

## D-WL1: approved security patch exception

The owner explicitly answered **“Allow the two security updates”** on
2026-10-20 after the existing lockfile failed the advisory gate for
RUSTSEC-2026-0381 and RUSTSEC-2026-0382. This supersedes the Stage 2a plan's
byte-identical Cargo.lock constraint for exactly these patch upgrades:

| Package | Before | Approved replacement | Source / license evidence |
|---|---|---|---|
| rustls-webpki | 0.103.15 | 0.103.16 | crates.io; `cargo info rustls-webpki@0.103.16`; ISC; official source https://github.com/rustls/webpki |
| webpki-roots | 1.0.9 | 1.0.10 | crates.io; `cargo info webpki-roots@1.0.10`; CDLA-Permissive-2.0; official source https://github.com/rustls/webpki-roots |

Source/license metadata was checked before updating the lockfile. Both licenses
are already in deny.toml's allowlist. Cargo update changed only these package
versions and their registry checksums; no dependency or workspace edge was
added. The post-update `cargo deny check licenses bans advisories sources`
passes. Final regression and fresh cleanroom review remain required.
Imported specification baselines and their original manifest are unchanged.

## Verification checkpoint before diagnostic fixes

At committed implementation revision `2513967cd1b139e382521ceaf1e8d4f10c32b9ca`,
`cargo test --workspace --locked -- --test-threads=4` completed with exit 0,
including all 52 byte tests, 25 namespace tests, 32 control-plane transaction
tests, 19 service unit tests, the five OpenAPI tests, both model suites, binary
restart coverage and provenance/156-column gates. Fmt and workspace/all-target
clippy with warnings denied also passed. This does not invalidate the reviewer's
subsequent independently reproduced three-party lock-cycle defect; that
schedule was not covered by the passing tests.

After the two approved security patch upgrades,
`cargo test -p anchorfs-backend -p anchorfs-core -p anchorfs-mount --locked`
completed with exit 0, as did cargo-deny's licenses/bans/advisories/sources gate.
Full post-fix/post-patch workspace verification and hosted CI remain pending.

The first review pair is diagnostic only: the coordinator shared peer findings
during the review, and both artifacts disclose the procedural limitation.
Their findings are retained rather than being presented as final independent
cleanroom verdicts. A fresh pair without peer conclusions is required after
corrections. Canonical `fix-review --auto` classified six deduplicated actions
as needs-manual under its authority/secret/config-file guardrail and made no
automatic implementation edits. The owner was asked for an explicit exception;
no gauntlet checkpoint commit or push has occurred.

The approved security-update provenance records were reconciled with the
lockfile. `cargo test -p anchorfs-conformance --test provenance --locked`
subsequently passed all three checks: recorded dependency versions, workspace
crate edges and fixture image identities. `git diff --check` passed.


## D-WL2: review-fix authorization and lock/deadline correction

The owner explicitly instructed **“Apply all review fixes and continue
autonomously through the gauntlet and PR”** on 2026-10-20. This overrides the
canonical fixer's sensitive-file auto-fix guard for this feature's review fixes;
no further approval is required for those fixes or the authorized PR workflow.

The diagnostic FK-cycle finding is corrected by retaining the plan's workspace
FOR UPDATE lock for lease transitions and taking workspace FOR SHARE as the
first lock inside objects::stage, before its reservation lock and INSERT's
implicit workspace FK key-share lock. Admission already has the shared lock;
standalone streamed replacement staging now follows the same order. The lock
spans only a short metadata transaction, never body/backend I/O. All production
objects::stage callers were audited: admission in pipeline/common.rs and the
standalone per-chunk transaction in pipeline/replace.rs. Callers already
holding operation/reservation locks must have acquired the workspace lock first.
This is an independent local lock-order correction, not a provider assumption.

Lease routes reject exhausted response budgets before preauthentication,
before exclusive workspace acquisition, after authority waits, and inside the
fresh receipt mutation closure after any receipt wait. This prevents detached
pool/lock waiters from starting a grant after their response deadline. The
small final check-to-commit window remains documented; no cancellation or
rollback-after-COMMIT guarantee is introduced.

New regressions are `object_staging_receipt_fence_and_lease_have_no_fk_lock_cycle`
(actual staging/fence/receipt/grant helpers and terminal quota assertions) and
`lease_pool_wait_past_deadline_does_not_create_an_abandoned_grant` (HTTP pool
saturation, detached completion, no receipt/epoch change, early-release control).
The HTTP eight-acquirer test now observes eight blocked workspace requests
before releasing its shared-lock holder. Timer cells unrelated to leases and
the namespace guard owner are restored; readRange declares its observed 409
fenced response. Final passing results and fresh review gates remain pending.
