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
The server file loader reads bytes then delegates to the common PEM parser so
file and in-memory fixtures use exactly the same validation path.

## Independent reviews and delivery

Pending the ordered gauntlet: two fresh independent code reviewers followed by
a fresh adversarial thermonuclear rubric review. Every reviewer will receive
an explicit approved-input packet and report a provenance verdict separately
from technical correctness. Prior plan-review conclusions will not be supplied.
No review, push or PR is claimed complete yet.

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
