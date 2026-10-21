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

## Kernel integration checkpoint (in progress)

The first host kernel run (6.8.0-124-generic) passed 15 of 21 selected kernel
cases, including Git smoke under a lease, 128 append records, explicit unsupported
operations, partial writes, two-process visibility, revocation, the lease race
and SIGTERM with a helper holding an open file. Two timer cases were excluded
from that iteration. Six failures are being corrected; this is not a passing
milestone gate. Four failures were test SQL names inconsistent with migrations.

The two orphan-metadata failures showed that Linux can omit FUSE_GETATTR_FH on
fstat after unlink. The implementation now resolves getattr through an already
open server handle for the requested inode when the kernel omits fh; it checks
the returned inode identity. It never re-resolves a detached name. This is an
independent correction based on the synthetic real-kernel test, extending the
WP3 fh-only assumption. Path-based mutation prechecks remain in place.

`flush` acknowledges synchronous writes without an extra remote operation,
including on a fenced mount, so close(2) does not invent an error after the
caller already observed the write/fsync outcome. This follows WP3's callback
table and is an exception to its broad fence-callback sentence.

Official Linux I/O-mode documentation consulted on 2026-10-21:
https://docs.kernel.org/filesystems/fuse/fuse-io.html. The strict profile selects
FOPEN_DIRECT_IO and leaves shared-mmap and writeback-cache capabilities disabled.
The overview's https://docs.kernel.org/filesystems/fuse.html URL returns 404;
this does not affect the dependency source or executable kernel tests.

The corrected host kernel checkpoint passed 19 tests (0 failures; 4 filtered)
in 182.79 s, including all earlier failures. Git and append had passed in the
first iteration; real renewal and SIGSTOP expiry remain unrun at this checkpoint.
The full local macOS workspace suite also passed; final gates will be repeated
on the committed kernel-suite revision after lint cleanup.

A fourth client conformance case, `client_resolves_deadline_after_publication`,
passed: each of three committed HTTP responses is delayed past the injected
client deadline; the same key resolves to one receipt and one directory.
The relay retains only request lines and idempotency keys, never credential
headers. Read requests explicitly accept binary byte responses, and tracing
records the returned trace id without request headers or bodies.

## Full-suite timer diagnosis

The first full host run passed renewal over 65 seconds with 20/40-second
handle/session TTLs. Its SIGSTOP case hung in the harness's external signal
helper; the stopped mount was confirmed through /proc and resumed from the
separate authorized SSH session. The watchdog had already expired, so that run
is not a passing gate. The harness now sends signals directly with the existing
nix dependency and kills only its paused test child before spawning timeout
cleanup helpers. This avoids fork/exec flushing inherited FUSE descriptors
while that filesystem is stopped. No production signal handling or timeout is
weakened. The full suite will be rerun after the affected run terminates.

Additional acceptance assertions cover server-clamped two-entry directory pages,
readRange generation versus the committed receipt, no chown metadata receipt,
no receipt of any state after stale ancestor rejection, and advancing session
expiry as well as advancing handle expiry during renewal.

The plan-fidelity audit tightened bound fencing to set the latch immediately,
before any await, without an ordinary-mount root probe. M9b now asserts no root
probe accompanies the fenced mutation as well as no requests after it. Startup
logs the renewed session expiry, and API failure events carry the configured
max_io_bytes for diagnosing server-limit mismatches. The Linux spawn API stays
target-gated; non-Linux CLI execution reports UnsupportedPlatform and the
portable mount_options signature is unchanged.

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

Read replies now recheck the latch after their remote request, and each read/write
slice checks current mount/handle state. An in-flight read that completes after
another callback latches the bound mount must not expose its late result;
`bound_fence_rejects_inflight_read_result` uses a deterministic BeforeGet gate to
exercise that schedule. Successful committed write results retain their byte
counts. The client also supports restoring persisted visible-ASCII idempotency
keys, with malformed header values rejected before any request.

A timestamp boundary audit inspected the already approved `time` 0.3.55
SystemTime interop API. Mount conversion now uses the checked nanosecond
constructor and validates years before any truncate/metadata mutation, rather
than risking a conversion panic. Linux unit and kernel setattr cases exercise
the year-10000 rejection. The selected dependency API/source is an approved
input; no dependency edge or registry identity changes.

At commit `05cf134`, Linux workspace fmt/clippy/build and the full workspace
suite passed. The serial mount suite passed 23 tests (22 acceptance scenarios
plus its helper target), no failures/skips, in 396.94 s on 6.8.0-124-generic.
The same mount suite also passed inside the workspace run with test-threads=4.
Private read-only mmap succeeded; shared writable mmap failed with ENODEV (19).
The final fencing/timestamp audit additions are being verified separately before
review; both gauntlet checkpoints will rerun the full applicable gates.
