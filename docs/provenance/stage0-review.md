# Stage 0 implementation and review record

Status: implementation verification in progress; fresh architecture review and repository-owner sign-off pending. This is procedural engineering evidence, not legal clearance or proof of erased historical exposure.

## Inputs and origin

Approved inputs: the seven docs/spec files pinned by import-manifest.json; docs/cleanroom.md; the repository-local Stage 0 implementation plan; independently authored AnchorFS code/design/tests; official Rust dependency metadata/APIs; official PostgreSQL/Linux/FUSE platform sources. The implementer did not access competitor code, SDK implementations, binaries, private protocols/traces or the excluded research archive. The original requirements/bootstrap author's public-document exposure remains disclosed in cleanroom.md.

Core, persistence, service, backend/mount and tests are independently authored from those inputs. All fixture IDs, credentials, paths, payloads and failure cases are synthetic. Persistence and backend/mount work used separate agent contexts with explicit approved input packets; these are implementation collaborators, not independent reviewers. Local registry dependency code was consulted only for reviewed general-purpose dependency APIs (SQLx, Axum, Utoipa, AWS SDK, Fuser and Testcontainers). No competitor-derived fixtures or expressions were incorporated.

Official platform references consulted on 2026-10-17: PostgreSQL's own runtime settings via disposable PostgreSQL 17; the Linux FUSE documentation and v5.10 public UAPI header linked in contracts.md; libfuse's public project documentation. No platform source code was copied into AnchorFS.

Every direct dependency and CI action is recorded in dependencies.md. The full lock graph passed cargo-deny with the plan's narrow license allow list and no advisory ignores. Imported spec hashes matched on implementation entry; final revalidation remains required. The import manifest and imported baselines are unchanged.

## Implementation decisions and plan corrections

- Exact dependency patch pins were chosen from official crates.io metadata on 2026-10-17; the full graph resolves under Rust 1.95.0. The TOML dependency uses from_str for document parsing; Value::from_str parses a single TOML value in the selected major.
- Complete lazy AWS credential-chain and custom-CA configuration need direct aws-credential-types and aws-smithy-http-client APIs. Existing transitive versions became direct dependencies after origin/license verification and a passing cargo-deny check. No advisory ignore or license widening was used.
- Conformance declares the existing axum pin directly as a dev dependency for HTTP request/body test types, omitted from the plan's dependency list. Workspace crate edges remain exactly as planned.
- The deadline test is a service middleware unit test with a test-only slow route. The 11 router integration tests plus this unit test cover the 12 planned router scenarios without exposing a test route in production.
- Body-boundary integration tests use reduced configurable caps (32/64 bytes) to exercise exact N/N+1 behavior without allocating 256 MiB per case. Production defaults are separately pinned by limits/design tests; Axum's implicit 2 MiB limit is disabled.
- Timers U2 remain proposed defaults pending owner acceptance. TLS U1 and backend readiness deviations are explicit in contracts.md. No authentication or native storage operation is implemented in Stage 0.

## Verification evidence

Work-package commits and final revision are available in branch history. The initial workspace commit ca294cb passed all five CI jobs: https://github.com/mattsp1290/anchorfs/actions/runs/39054969761 . Final implementation CI is still required.

Local environment: Rust 1.95.0; macOS development host; Docker Engine 29.2.1. Tests create disposable containers using the official multi-platform image postgres:17@sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3. Testcontainers' module fsync-off default is explicitly disabled; unsafe-setting tests override it intentionally. No existing infrastructure data is touched. Docker Engine is a host runtime rather than a container image; no engine image digest is applicable.

Observed work-package checks: core 7 tests; persistence 12 tests; backend/mount 9 macOS tests; service 1 middleware + 6 binary + 3 OpenAPI tests; conformance 11 router + 3 design + 2 provenance tests. Final full-workspace test, fmt, clippy, build and deny results will be recorded after integration. Linux runtime tests and arm64 builds are CI responsibilities, not inferred from macOS success.

## Fresh architecture review

Pending. Required scope: all persistent columns, B1–B13 crash boundaries, lock order/modes, fencing, independent naming/design, dependency origins/licenses, source updates, fresh input context and unsupported claims. Technical and provenance verdicts must be separately recorded with revision and limitations.

## Repository owner sign-off

Pending — the owner must accept the final Stage 0 architecture record and timer defaults. This line cannot be supplied by an implementation agent. Stage 1 planning is gated until sign-off.
