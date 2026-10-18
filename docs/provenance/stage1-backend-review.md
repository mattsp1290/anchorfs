# Stage 1 backend implementation evidence

## Inputs and origin

Implementation follows the reviewed local plan `.agents/plans/stage1-backend-minio-contract/`, approved `docs/spec/02-storage.md` (object namespace, upload uncertainty, verified reads), `03-interfaces.md` (dependency and integrity errors), and `04-deployment.md` (backend configuration and readiness), with independent design decisions E1–E14 recorded in that plan. The original imported specifications and manifest remain unchanged.

New backend algorithms, synthetic fixtures, and tests are independently authored from these requirements and general-purpose pinned crate APIs. The implementer inspected local aws-sdk-s3 1.152.0 configuration/error reexports and aws-smithy-runtime-api 1.19.0 response construction APIs (Apache-2.0 dependency sources already recorded in dependencies.md). No competitor implementation, SDK, private trace, or external research archive was consulted. Fresh reviewer context is a procedural boundary, not legal clearance or proof of erased historical exposure.

MinIO use is authorized by user decision E1 on 2026-10-17 after the plan's image-availability diagnosis. Its digest, source and license boundary are recorded in dependencies.md before use. Only disposable containers and loopback fault proxies are used; existing infrastructure is untouched.

## Implementation decisions

WP1 implements the plan's two-upload reconciliation bound, prefix guards, length/digest validation, configurable checksum header, and retry-free list probe. No deletion, multipart, metadata publication, or native `/v1` operation is introduced. UUID parsing validates canonical lowercase hyphenated text directly, without a new dependency. SDK raw response construction is available through the HttpResponse reexport with inferred status/body types.

## Verification so far

- `cargo check -p anchorfs-backend -p anchorfs-service --locked`: passed.
- `cargo test -p anchorfs-backend --locked`: nine network-free tests passed on the development macOS host, including service, construction, timeout, dispatch and response-error classification.
- `cargo deny check licenses bans advisories sources`: passed all four checks with existing duplicate-version warnings, without policy changes.

## Provider observations and plan corrections

The fixture and transparent-proxy gates each passed three times before scenario verification. The first complete F1–F15 run passed 11 grouped tests, but repeated workspace runs exposed a startup race: observed Bitnami logs show a temporary setup server stopped about five seconds after startup before the final `** Starting MinIO **` marker. Two successful HEAD requests did not prove setup had ended. The fixture now waits for that final marker before the bounded list poll, bucket creation and two HEAD checks; the corrected gates each passed three additional runs. This is an independently observed provider lifecycle correction, not a change to product scope.

The interrupted-body proxy delays its stream error by 50 ms after yielding bytes, so headers/body reach the client before interruption; otherwise hyper can fail before headers and the SDK correctly retries the GET. CI test concurrency is capped at four per test binary, as the plan permits for container pressure. A local full run at default concurrency failed before the startup race was diagnosed; no unbounded-concurrency claim is made.

Observed pinned image: `bitnamilegacy/minio:2025.7.23-debian-12-r5@sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20`. `minio --version` reports `DEVELOPMENT.2025-07-23T15-54-02Z`, commit `7ced9663e6a791fef9dc6be798ff24cda9c730ac`, Go 1.24.6 linux/arm64, GNU AGPLv3. F11b rejects a wrong checksum with `XAmzContentChecksumMismatch` (mapped to IntegrityMismatch). F15 clamps the range with HTTP 206 and Content-Range `bytes 9000-9999/10000`.

Development toolchain: rustc 1.95.0 (59807616e, 2026-04-14), cargo 1.95.0, Docker Engine 29.2.1, macOS arm64. Local service conformance passed 17 tests; service binary tests passed eight, including development successful readiness, durable failed backend readiness with live process/healthz, and plaintext rejection. Design coverage and provenance suites passed three tests each. The macOS Docker-free core/backend/mount subset passed. Relative links under docs resolved, imported specifications remain unchanged, and the OpenAPI command still matches the frozen snapshot.

## Review and acceptance status

Local implementation verification passed: `cargo test --workspace --locked -- --test-threads=4`, `cargo fmt --all --check`, and workspace Clippy with `-D warnings`. The final checksum signing assertion and the corrected 404 mapping also passed targeted tests. Fresh independent code reviews, the review gauntlet and final CI evidence remain pending. Owner sign-off is pending.

## Limits and follow-ups

The pinned Bitnami legacy repository may disappear; mirroring redistributes AGPL software and requires a separate license review. Anonymous registry pull limits remain an operational risk. TLS U1 and durable-profile end-to-end successful backend readiness remain unverified. Checksum `none` relies on transport integrity for successful uploads. Readiness lists under the managed prefix and does not test write permission. No Spaces, AWS, multi-node durability, or performance acceptance follows from this milestone.
