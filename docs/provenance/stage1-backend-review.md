# Stage 1 backend implementation evidence

## Inputs and origin

Implementation follows the reviewed local plan `.agents/plans/stage1-backend-minio-contract/`, approved `docs/spec/02-storage.md` (object namespace, upload uncertainty, verified reads), `03-interfaces.md` (dependency and integrity errors), and `04-deployment.md` (backend configuration and readiness), with independent design decisions E1–E14 recorded in that plan. The original imported specifications and manifest remain unchanged.

New backend algorithms, synthetic fixtures, and tests are independently authored from these requirements and general-purpose pinned crate APIs. The implementer inspected local aws-sdk-s3 1.152.0 configuration/error reexports and aws-smithy-runtime-api 1.19.0 response construction APIs (Apache-2.0 dependency sources already recorded in dependencies.md). No competitor implementation, SDK, private trace, or external research archive was consulted. Fresh reviewer context is a procedural boundary, not legal clearance or proof of erased historical exposure.

MinIO use is authorized by user decision E1 on 2026-10-17 after the plan's image-availability diagnosis. Its digest, source and license boundary are recorded in dependencies.md before use. Only disposable containers and loopback fault proxies are used; existing infrastructure is untouched.

## Implementation decisions

WP1 implements the plan's two-upload reconciliation bound, prefix guards, length/digest validation, configurable checksum header, and retry-free list probe. No deletion, multipart, metadata publication, or native `/v1` operation is introduced. UUID parsing validates canonical lowercase hyphenated text directly, without a new dependency. SDK raw response construction is available through the HttpResponse reexport with inferred status/body types.

## Verification so far

- `cargo check -p anchorfs-backend -p anchorfs-service --locked`: passed.
- `cargo test -p anchorfs-backend --locked`: nine network-free tests passed on the development macOS host; further classification coverage is being finalized.
- `cargo deny check licenses bans advisories sources`: passed all four checks with existing duplicate-version warnings, without policy changes.

## Review and acceptance status

Implementation and verification are in progress. Fresh independent code reviews, the review gauntlet, provider observations, full workspace verification, and final CI evidence remain pending. Owner sign-off is pending.

## Limits and follow-ups

The pinned Bitnami legacy repository may disappear; mirroring redistributes AGPL software and requires a separate license review. Anonymous registry pull limits remain an operational risk. TLS U1 and durable-profile end-to-end successful backend readiness remain unverified. Checksum `none` relies on transport integrity for successful uploads. Readiness lists under the managed prefix and does not test write permission. No Spaces, AWS, multi-node durability, or performance acceptance follows from this milestone.
