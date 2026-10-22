# Stage 2b Linux mount implementation and verification

The client, timestamp amendment, Linux mount and kernel suite are implemented on
`feat/stage2b-linux-mount`. Verification below names the actual source revisions;
the independent review gauntlet and both push checkpoints have passed. Draft
[PR #7](https://github.com/mattsp1290/anchorfs/pull/7) is open; all five hosted jobs
passed on the source checkpoint below.

## Approved inputs and independent design

Implementation began from `1211158b6ee35e083e9785b1b181ceade4d441d6` on 2026-10-21.
Before editing, all seven imported specification hashes and the overview's
contracts, persistent-state, OpenAPI, dependency, lockfile and CI identities
matched. The original imported specs and import manifest remain unchanged.
A hash proves identity, not originality or legal suitability.

| Work | Approved requirement / source |
|---|---|
| Partial native client, authority headers and receipt recovery | spec 02 retry receipts; spec 03 native API; AnchorFS route annotations, DTOs and receipt implementation; plan WP1 |
| Explicit atime/mtime | spec 03 mode/owner updates and filesystem behavior; explicit owner decision D7; existing inodes columns; independent local amendment in contracts.md; WP2 |
| Shared and lease-bound FUSE mount | spec 01 R1/R3/R5/R6, spec 02 strict reads/cache profile, spec 03 filesystem/error/workflow contract, spec 07 worker lifecycle; WP3 |
| Kernel suite and Git smoke | spec 05 Stage 2 and T5/T7/T12/T17/T19; spec 07 cases 1–2 and 5–6; independently authored synthetic tests; WP4 |

New code and fixtures are independently authored from those inputs, AnchorFS's
own code/tests/designs, and approved general-purpose dependency/platform APIs.
The wire shapes derive from AnchorFS `dto.rs`, not another vendor's SDK.
The fuser 0.18.0 public trait/configuration API and time 0.3.55 SystemTime
interop source were inspected in the local registry. No competitor client,
implementation, SDK, binary, private protocol, trace or research archive was
consulted. Fresh review context is procedural separation and is not legal
clearance or an assertion that historical exposure was erased.

Official kernel I/O reference consulted on 2026-10-21:
https://docs.kernel.org/filesystems/fuse/fuse-io.html. The overview's older
https://docs.kernel.org/filesystems/fuse.html URL returned 404; no behavior was
inferred from that failed fetch.

## Dependency and tool provenance

Before promotion, `cargo info reqwest@0.12.28` confirmed MIT OR Apache-2.0 and
https://github.com/seanmonstar/reqwest; `cargo info libc@0.2.191` confirmed the
same license expression and https://github.com/rust-lang/libc. Existing fuser
is MIT; time and the remaining workspace inputs retain their approved licenses.
The exact registry package/version/source/checksum sets in Cargo.lock equal
main's. Changes add only the client workspace member and dependency edges.
There is no migration; persistent coverage remains 156 columns.

`dependencies.md` records runtime reqwest, Linux libc, the shipped webpki trust
roots and test fixture roles. fusermount3 and git are external processes, never
linked, vendored or distributed here. The Linux test target reuses the existing
MinIO fixture and independently authored API relay by path. Container images
retain their approved immutable digests; fixtures never use infrastructure data.

## Executed verification

- Client units: headers/plaintext policy, core errno hints and persisted key
  validation/identity round trip passed. Client conformance: four cases passed
  against disposable PostgreSQL/MinIO, including all three response attempts
  lost for mkdir/open/write/append/session mint, and a separate committed
  response delayed past every injected client deadline. Typed receipt lookup
  returns the original result and SQL proves a single logical mutation.
- Timestamp conformance: two cases passed (owner/admin/forbidden, invalid years,
  symlink times, generation/ctime, replay/digest mismatch and live lease gate).
  The persistence timestamp/empty-update test passed. OpenAPI snapshot: five
  passed; binary suite: twelve passed, including process-restart byte persistence.
- `05cf134`: full Linux workspace fmt, all-target clippy, build and workspace
  tests passed. Serial mount suite: 23 tests (22 acceptance scenarios plus its
  helper target), zero failures/skips, 396.94 s on kernel 6.8.0-124-generic.
  The suite also passed within workspace tests using test-threads=4.
- Host observations: shared writable mmap failed with ENODEV (19); private
  read-only mmap succeeded, including on Git's index. Git init/add/two commits,
  clean status, log, branch checkout/switch and committed blob inspection passed.
  Per-command wall times are in the host log and are smoke observations only.
- `ef2b9f0`: Linux all-target workspace clippy passed; mount units 14/14, CLI
  2/2 and affected kernel cases 20/20 passed in 110.02 s. The subset excluded
  only previously passing Git, append and the two long timer scenarios. New
  coverage includes late read suppression, immediate bound fencing without
  a root probe, checked timestamp bounds and positive rmdir/regular mknod.
- Full macOS workspace tests, workspace build, all-target clippy, all-target
  compile-only tests and the required core/backend/mount subset passed.
  `cargo deny check licenses bans advisories sources` passed with existing
  duplicate-version warnings. Final gauntlet checkpoints rerun applicable gates.

The Linux host readiness checks observed a non-root uid, /dev/fuse, fusermount3
3.14.0, Docker 29.2.1 and local temp/checkout/target filesystems. Host identities,
credentials and NFS paths are not recorded here. The workflow is documented in
`docs/ops/linux-host-verification.md`.

## Independent corrections and recorded deviations

- Executable routes/OpenAPI take precedence over WP1's shorthand: unlink and
  rmdir take path as a query parameter, not a DELETE JSON body.
- Linux can omit FUSE_GETATTR_FH on fstat after unlink. Getattr can use an
  existing live server handle for that exact inode, with an identity check;
  it never re-resolves a detached name. Kernel orphan tests exercise this.
- Fresh lookups promote their verified hard-link alias so a remotely removed
  older alias cannot keep breaking opens through a surviving link. M18 covers
  cross-mount removal; the portable table test covers re-observation.
- A bound fenced mutation latches before any await. Read replies and subsequent
  slices check the latch; a deterministic BeforeGet schedule verifies that a
  late read result is rejected. Committed write slices keep accurate byte counts.
- Timestamp conversion uses a checked nanosecond constructor, validates years
  before any truncate/metadata mutation, and returns EINVAL without panicking.
- Flush succeeds for synchronous writes, including when fenced, so close does
  not invent another write error. This follows WP3's callback table and is an
  exception to its broad fence-callback sentence.
- Test signals use existing nix calls directly. Fork/exec with an open descriptor
  into a SIGSTOP-paused FUSE mount could block the old external signal helper.
  The watchdog kills only its paused child before invoking cleanup helpers;
  the corrected SIGSTOP case passed independently and in the full suite.
- Production unmount is non-lazy, keeps serving on refusal and polls its helper
  while listening for a second shutdown signal. Linux spawn stays target-gated;
  the portable mount_options signature and non-Linux CLI failure remain intact.
- Post-startup 401 maps to ESTALE (the recorded spec 03 table deviation).
  Ordinary fenced publications probe session validity; workspace busy does not
  latch. In-process rebind, server parent-inode guards and exercised HTTPS remain
  deferred as agreed in the plan.

## Independent reviews and delivery

Three fresh-context Opus code reviews ran against an explicit approved-input
packet and full branch diff/files. Kernel Correctness and Boundary
Maintainability independently returned `REQUEST_CHANGES` and **no blocking
provenance findings** for `015b84e04ec70d542edd23f90875bbb28557f98e`.
The standard pass dispositioned 22 deduplicated items: 19 fixes (one automatic,
18 manually approved) and three skipped/already-satisfied items with recorded
rationale. TTL clock comparison and lease polling conflicted with the approved
D14/D8 decisions; their limits remain explicit. Both reviewers received the
approved requirements packet and full code, but did not read the normative
plan text. This context limitation is preserved; it is not retroactively
claimed to be a complete plan review.

The standard checkpoint is
`47b1bf6c926c662cfabaf3d5ccae5b547b467bf3`; its local and Linux gates passed,
and the first push's remote SHA matched exactly. The initial canonical auto
summary was fixed:1/skipped:1/needs-manual:20. After the owner's explicit manual
approval, auto revalidation was fixed:0/skipped:22/needs-manual:0. No protected
file was silently auto-edited.

Structural Quality independently reviewed `47b1bf6` using the complete rubric
pinned from cursor/plugins at
`699e1e7488c53c05ca13409f40da83cd37682091`:
https://raw.githubusercontent.com/cursor/plugins/699e1e7488c53c05ca13409f40da83cd37682091/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md.
It returned `REQUEST_CHANGES` and **no blocking provenance findings**: eight
Important findings and twelve Suggestions. It received the normative plan,
approved packet and full current source, without the other reviewers' findings.
All twenty items were dispositioned under the existing manual approval. The
reservation-expiry message already had an end-to-end contract assertion;
a typed reason/distinct wire code is explicitly deferred. The original auto
summary fixed:0/skipped:0/needs-manual:20 remains preserved; final auto
revalidation was fixed:0/skipped:20/needs-manual:0.

A labeled recheck of that review's own findings returned **APPROVE** for code
structure and **no blocking provenance findings**. Its two remaining Suggestions
(formatting and typed Accept selection) were then fixed and the client suite
rerun. This is a recheck, not a fourth independent review or certification of
test results. The thermonuclear checkpoint is
`21ab15b54b39af5f9e6a34cf74eb5982b5344b7a`; the separate regular push's remote SHA
matched exactly after all final gates passed.

Final local gates on the checkpoint candidate passed: fmt, all-target workspace
clippy, cargo-deny licenses/bans/advisories/sources, workspace build and no-run,
client six units and the macOS core/backend/mount subset. Final Linux gates
passed fmt, all-target clippy, build, serial mount tests **29 passed / 0 failed /
0 ignored in 397.46 s**, and full workspace tests with test-threads=4. The latter
included the same **29 passed / 0 failed / 0 ignored in 399.82 s**. These are
28 acceptance scenarios plus the helper target, not 29 independent scenarios.
The final source manifest SHA-256 was
`b03b979b2f288abb9fcbfe735abc5fe7fc3d0fdeaeedd9e96c4a380d9da8b65e`;
it was checked on the host before execution and matched the committed candidate.
Client conformance 5, namespace 28 (including both time cases), design coverage 3,
provenance 3, service 17, binary 12 and OpenAPI snapshot 5 passed. This verifies
38 implemented routes / 0 stubs, API version `1.0.0-stage2b`, 156 persistent
columns and the amended dependency/fixture gates. No AnchorFS mount or disposable
MinIO fixture remained after the test reaper completed.

Ignored local artifacts are retained under:

- `reviews/feat-stage2b-linux-mount-2026-10-21-134637-015b84e04ec7`
- `reviews/feat-stage2b-linux-mount-2026-10-21-195019-47b1bf6c926c`

They contain the immutable original review files, manifests, summaries, manual
authorization/dispositions, recheck and separate checkpoint receipts. They are
not bundled into the repository, consistent with its review-artifact convention.
No review verdict implies legal clearance or erased historical exposure.

Hosted workflow [39704959806](https://github.com/mattsp1290/anchorfs/actions/runs/39704959806)
for source revision `21ab15b54b39af5f9e6a34cf74eb5982b5344b7a` passed all five jobs:
`lint`, `deny`, `test`, `build-linux-arm64` and `build-macos`. The test job
119582733047 recorded Linux **6.17.0-1022-azure**, fusermount3 **3.14.0**,
non-root uid **1001**, and **29 passed / 0 failed / 0 ignored in 224.12 s**
for the real FUSE target. No fallback skip was introduced. This covers the
hosted branch source at the named revision; the PR's current check suite and
local delivery receipt identify the final evidence-only revision's result.
Hosted logs are dated 2026-10-22 UTC (2026-10-21 in the session's timezone).

## Limits carried from the approved plan

- single-node only (M3 proves API-mediated visibility between two processes on one kernel, not cross-node behaviour)
- path-based operations keep a time-of-check/time-of-use window between the identity pre-check and the server's lookup (handle operations do not)
- `touch` by a non-owner writer fails
- `UTIME_NOW` uses the client clock
- a `SIGKILL`ed self-minting mount leaves a session until its TTL
- `MAP_PRIVATE` pages mapped before a fence stay readable
- the `https`/`--ca-bundle` path is unexercised
- `allow_other` kernel path untested
- a `401` after startup maps to `ESTALE` (deviation from the spec 03 table)
- no kernel caching, so every syscall is at least one API round trip and read amplification is one object per touched chunk
- no mmap claim
- Git success is a smoke observation
- `statfs` reports no capacity
- `fstat` on open files costs a receipt-free `fsync` round trip
- renewal is best-effort and a long pause still fences (by design)
- arm64 FUSE untested
- TLS U1 open (mount tests use `--allow-plaintext` on loopback)

The plan's “every syscall” shorthand above describes remote file and metadata
operations. Unsupported callbacks, statfs, flush, forget and iteration over an
already captured directory snapshot do not require an API request. This is
not a latency or object-request-count guarantee for arbitrary workloads.
Self-minted sessions left by forced termination can consume the credential's
64-session bound until expiry; production hosts should pre-mint sessions.

Additional reviewed limits: a combined setattr may truncate before a later
attribute update fails; a path identity mismatch can return ESTALE after a
namespace mutation has committed. Default data-call retry budgets are 183 s,
with up to 33 s of receipt resolution; callbacks issuing multiple API requests
can take longer. These are bounded retry defaults, not syscall latency promises.
