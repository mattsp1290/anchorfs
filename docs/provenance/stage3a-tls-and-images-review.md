# Stage 3a TLS and container image evidence

## Approved inputs and independent design

Implementation began from clean main `c6704036650b0e6e267dd798a3565bbcd37a3eef`
on `feat/stage3a-tls-and-images`. All seven imported specification SHA-256
values matched the unchanged import manifest before edits. The owner requested
the reviewed Stage 3a plan, review gauntlet, and a PR.

| Work | Approved requirement / independent decision | Origin |
|---|---|---|
| TLS listener, fail-closed startup | spec 03 HTTP API TLS; spec 04 deployment; Stage 3a D1–D6 | independently authored AnchorFS integration using chosen general-purpose library APIs |
| synthetic PKI, client and mount verification | spec 04 verified TLS; Stage 3a D7–D9, D13–D17 | synthetic per-test rcgen CA, no copied fixtures, private keys generated only in temporary directories |
| native images and smoke | spec 04 amd64/arm64; spec 05 Stage 3; Stage 3a D10–D12 | independently authored Dockerfile/jobs, official Docker library metadata |
| records | cleanroom checklist and Stage 3a WP4 | repository execution evidence |

No competitor implementation, SDK, binary, trace or research archive was
consulted. Imported specifications and import manifest are unchanged. Standard
TLS/HTTP/FUSE names express standards, without private compatibility claims.
Fresh review context provides procedural separation, not proof of legal clearance
or erased historical exposure.

## Dependency and tool provenance

Exact-version cargo info checked axum-server 0.8.0 MIT, rustls 0.23.46
Apache-2.0 OR ISC OR MIT, rustls-pki-types 1.15.1 MIT OR Apache-2.0,
rcgen 0.14.10 MIT OR Apache-2.0, fs-err 3.3.2 MIT OR Apache-2.0,
pem 4.0.0 MIT and yasna 0.6.0 MIT OR Apache-2.0 on 2026-10-22.
Local registry manifests/signatures verified provider features, PEM APIs,
rcgen SAN/signing APIs, axum-server HTTP/1-only, acceptor timeout and handles.
See [dependency provenance](dependencies.md) for the complete package table.

The actual Cargo.lock adds 18 registry packages rather than the anticipated
five. Work stopped on that dependency gate while exact-version cargo info
checked and recorded every additional source/license. These include optional
rcgen x509-parser resolution; `cargo tree --locked -i x509-parser` reports
no active edge. No existing registry version changed. This is the plan's
explicit stop-and-record exception, without changing the license allow list.

`docker buildx imagetools inspect rust:1.95.0-bookworm` resolved manifest-list
digest `6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1`;
`debian:bookworm-slim` resolved
`7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8`.
Both index outputs contain linux/amd64 and linux/arm64/v8 platform entries.
The Docker server reported 29.2.1 locally. Base image licenses and fuse3
runtime license notes are in dependencies.md; no registry publication occurs.

## Executed verification

Initial macOS `cargo check --workspace --all-targets` passed.
`cargo test -p anchorfs-service --test binary --locked tls -- --test-threads=4`
passed all four new TLS binary tests (10.93 s), with disposable data only.
`cargo deny check licenses bans advisories sources` passed all four gates.
The following macOS gates passed on the implementation worktree before the
initial commit:

- `cargo fmt --all --check`, `git diff --check`; changed-doc relative links resolve.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- `cargo build --workspace --locked` and `cargo test --workspace --no-run --locked`.
- `cargo test -p anchorfs-core -p anchorfs-backend -p anchorfs-mount --locked`
  (48 tests; Linux kernel target has zero tests on macOS).
- `cargo test -p anchorfs-service --test binary --test openapi_snapshot --lib --locked`
  (21 unit + 16 binary + 5 snapshot tests).
- `cargo test -p anchorfs-conformance --test client --test provenance --test design_coverage --test service --locked`
  (6 client + 5 provenance + 3 design coverage + 17 service tests).
- Initial `docker build --progress plain -f docker/Dockerfile --target service -t anchorfs-service:ci .`
  passed on linux/arm64: shared builder release 3m17s (198.7 s build step).
  Slim Debian useradd and ca-certificates install succeeded without additions
  to the prescribed runtime packages. Mount target/smoke remain in progress.

Linux host and hosted gates are in progress; see
[acceptance map](stage3a-tls-and-images-acceptance.md).

## Corrections and deviations

Tests use separate binary TLS modules to keep the existing binary suite readable.
The native client fixture initially used an absolute root path, rejected by the
existing API; it was corrected to the root-stat API (`stat(None)`) before rerun.
A narrow explicit D13 deviation permits the service binary integration test to
name rustls/rustls-pki-types from the service's existing runtime dependencies
for verified keep-alive and HTTP/2 socket probes. Conformance and mount fixtures
still name only AnchorFS-owned TLS/handle types and gain no TLS-library edge.
The fresh adversarial reviewer checked and accepted this dependency boundary;
no package, wire or runtime policy changes result.
The server file loader reads bytes then delegates to the common PEM parser so
file and in-memory fixtures use exactly the same validation path.

## Independent reviews and delivery

Standard review of `04ce5da0b2d39f553dc4bb48a00d43e419bfe565` completed via
fresh independent Opus processes using the installed Claude CLI (no conversation
inheritance, auto-memory disabled, settings/plugins disabled, only Write tools
into assigned output directories). Each received the full diff, full changed
files, approved specifications, cleanroom policy and normative work packages;
no research archive or prior reviewer conclusions. Transport Correctness:
REQUEST_CHANGES, 2 Important + 7 Suggestions. Packaging and Acceptance:
REQUEST_CHANGES, 2 Important + 12 Suggestions. Both reported **no blocking
provenance findings**, with explicit limits on read-only packet evidence.

Artifacts (local, uncommitted):
`reviews/feat-stage3a-tls-and-images-2026-10-22-104303-04ce5da0b2d3/`,
reviewer slugs `transport-correctness` and `packaging-and-acceptance`.
The canonical dotfiles fix-review workflow selected all findings, merged five
duplicates and preferred Important severity. Its initial automatic pass held
configuration/TLS-related changes for manual inspection. The authorized
implementer applied these through the canonical fixer workflow; a subsequent
auto validation has no unresolved manual items.

Disposition: 17 distinct findings addressed (3 Important, 14 Suggestions),
including same-fixture trusted mount control, pre-minted positive session,
bidirectional visibility, secret-free refusal logs, exact TLS receipt counts,
HTTP/2 prior-knowledge refusal, explicit keep-alive TLS stream during shutdown,
reset/abort handling, I/O error kinds, fixed GID/smoke checks, common smoke
script retaining both required job IDs, architecture assertions for both
images and native CPU checks, one digest scanner, rendered dependency table,
CA-aware helper boundaries, feature/component evidence and license-label scope.
A requirements audit also added the planned createSession call over TLS.

One suggestion is deferred: adding a production X.509 parser and certificate
expiry warnings is outside the approved startup policy; certificate validity
continues to be enforced by verifying clients. No new runtime dependency or
startup semantics are introduced for this suggestion. The matrix suggestion
is addressed by a shared script while retaining the two job IDs mandated by
the plan. No Critical/Important finding remains unresolved.

Standard checkpoint gates passed on the corrected worktree:
`cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`;
`cargo deny check licenses bans advisories sources`; `cargo build --workspace --locked`;
`cargo test --workspace --no-run --locked`; `cargo test --workspace --locked -- --test-threads=4`
(341 tests, including doctests); and `git diff --check`.
The Linux-only kernel test target has zero tests on macOS. The first checkpoint committed the fixes as
`9aed65171b24222bd94c401e485c0cb3bf927534` and pushed successfully to origin;
`git ls-remote` confirmed the remote branch equals local HEAD.

Thermonuclear Maintainability independently reviewed that revision using the
complete live upstream rubric pinned at
`eb4db2b55b2b7dec3d9282754cddf0a56bf4ff10`:
https://raw.githubusercontent.com/cursor/plugins/eb4db2b55b2b7dec3d9282754cddf0a56bf4ff10/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md
The SHA was resolved from refs/heads/main after the first verified push, and
the fetched frontmatter name was validated. Verdict **APPROVE**, zero Critical,
Important or Suggestions, provenance **no blocking provenance findings**.

Fresh context was provided through an explicit evidence packet containing the
full code/diff, requirements, stats and history. The standard-review findings
and verdict section was withheld from the supplied provenance-record copy and
its diff under AGENTS.md's independent-context rule. Generic user dotfiles rules
for git, delegation and efficiency contain no research or reviewer findings;
no global CLAUDE.md or auto-memory was supplied to the standard reviewer
processes. The adversarial reviewer inherited no parent conversation. All
three stages are engineering reviews with stated evidence limits, not legal
clearance or proof of erased historical exposure.

Adversarial artifacts (local, uncommitted):
`reviews/feat-stage3a-tls-and-images-2026-10-22-110203-9aed65171b24-thermonuclear/`,
slug `thermo-nuclear-code-quality-review`. Canonical fixer invoked on that exact
manifest: `fixed:0 skipped:0 needs-manual:0`. The second checkpoint repeated all six local Rust gates above and
`git diff --check`: all 341 workspace tests passed again. Both Docker targets
rebuilt using cached builder layers and `bash ci/image-smoke.sh arm64` passed.
No implementation change was required by this review; this checkpoint records
its approval and the D13/documentation clarifications. Second push, PR and
hosted execution are pending.

## Limits carried forward

- Restart-only certificate rotation; no reload, client certificates or mTLS.
- TLS 1.3 and HTTP/1.1 only. Non-rustls clients untested; kubelet compatibility
  remains an assumption. HTTP/2 is disabled in the server, not merely in ALPN.
- Durable TLS readiness observes backend 503 against unreachable https S3;
  successful TLS readiness is tested with development profile/plaintext MinIO.
- Stuck handshakes use the acceptor timeout, not graceful shutdown interruption.
- Default durable loopback service container is unreachable externally until
  listen address and both TLS paths are supplied.
- Images build and smoke only: no registry push, multi-arch manifest, SBOM/signing.
- apt package versions unpinned; base images pinned by manifest-list digest.
- Container `/dev/fuse`, capabilities and propagation documented, not proven.
- No cluster, Helm, Spaces, cross-node or performance claims.
- Image build durations will be recorded as observations, not optimized.

## Additional local packaging observations

Both service and mount targets built on local linux/arm64 and the original
nine-item smoke script passed (fusermount3 3.14.0; image arm64). fuse3 installs
successfully; useradd exists in the slim base. The shared aws-lc-sys build output
reports `Building with: CC` for aarch64-unknown-linux-gnu; no builder package
addition was necessary. The initial build downloaded clippy due to the
rust-toolchain component list. The fixer sets `RUSTUP_TOOLCHAIN=1.95.0` in the
builder so release builds use the digest-pinned toolchain without that extra
component download. Rebuilt service and mount targets passed; release builder steps took 189.9 s
and 199.1 s respectively while tests were still being corrected between local
builds. Once source was stable the builder target was cached. Neither rebuild
log downloaded/installed toolchain components. `bash ci/image-smoke.sh arm64`
passed all checks, logging both images as arm64 and fusermount3 3.14.0.
The final Linux mount test target compile check also passed (55.27 s).
`docker run --rm anchorfs-builder:verify cargo check --tests -p anchorfs-mount
--release --locked` passed on Linux arm64 (36.42 s); this is compilation evidence,
not Docker-on-Mac kernel acceptance. The separate Linux host target is awaiting
owner input; no pre-existing infrastructure data has been used.
