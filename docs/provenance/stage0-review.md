# Stage 0 implementation and review record

Status: Stage 0 accepted by the repository owner. Local implementation verification, architecture recheck, both gauntlet review/fix stages and all five final-code checkpoint CI jobs passed. This is procedural engineering evidence, not legal clearance or proof of erased historical exposure.

## Inputs and origin

Approved inputs: the seven docs/spec files pinned by import-manifest.json; docs/cleanroom.md; the repository-local Stage 0 implementation plan; independently authored AnchorFS code/design/tests; official Rust dependency metadata/APIs; official PostgreSQL/Linux/FUSE platform sources. The implementer did not access competitor code, SDK implementations, binaries, private protocols/traces or the excluded research archive. The original requirements/bootstrap author's public-document exposure remains disclosed in cleanroom.md.

Core, persistence, service, backend/mount and tests are independently authored from those inputs. All fixture IDs, credentials, paths, payloads and failure cases are synthetic. Persistence and backend/mount work used separate agent contexts with explicit approved input packets; these are implementation collaborators, not independent reviewers. Local registry dependency code was consulted only for reviewed general-purpose dependency APIs (SQLx, Axum, Utoipa, AWS SDK, Fuser and Testcontainers). No competitor-derived fixtures or expressions were incorporated.

Official platform references consulted on 2026-10-17: PostgreSQL's own runtime settings via disposable PostgreSQL 17; the Linux FUSE documentation and v5.10 public UAPI header linked in contracts.md; libfuse's public project documentation. No platform source code was copied into AnchorFS.

Every direct dependency and CI action is recorded in dependencies.md. The full lock graph passed cargo-deny with the plan's narrow license allow list and no advisory ignores. Imported spec hashes matched on implementation entry; final revalidation remains required. The import manifest and imported baselines are unchanged.

## Implementation decisions and plan corrections

- Exact dependency patch pins were chosen from official crates.io metadata on 2026-10-17; the full graph resolves under Rust 1.95.0. The TOML dependency uses from_str for document parsing; Value::from_str parses a single TOML value in the selected major.
- Complete lazy AWS credential-chain and custom-CA configuration need direct aws-credential-types and aws-smithy-http-client APIs. Existing transitive versions became direct dependencies after origin/license verification and a passing cargo-deny check. No advisory ignore or license widening was used.
- Conformance declares the existing axum pin directly as a dev dependency for HTTP request/body test types, omitted from the plan's dependency list. Workspace crate edges remain exactly as planned.
- The deadline test is a service middleware unit test with a test-only slow route. The 11 original router integration tests plus this unit test cover the 12 planned router scenarios without exposing a test route in production; two additional router tests cover root selection and authority headers.
- Body-boundary integration tests exercise the production 4 MiB write cap, 4 MiB+1 replacement allowance and 256 MiB+1 rejection. Axum's implicit 2 MiB limit is disabled. Limits::default derives maximum file size with checked multiplication.
- The repository owner accepted the U2 timer defaults on 2026-10-17. TLS U1 and backend readiness deviations are explicit in contracts.md. No authentication or native storage operation is implemented in Stage 0.

## Verification evidence

Work-package commits and final revision are available in branch history. The initial workspace commit ca294cb passed all five CI jobs: https://github.com/mattsp1290/anchorfs/actions/runs/39054969761 . The standard-review checkpoint 05bd5abd89c5f78566e4189e54429b68b86e74cb also passed all five jobs: https://github.com/mattsp1290/anchorfs/actions/runs/39056801910 . The thermonuclear checkpoint bcb09dcdb72852a43b15b90828c2fa298513d965 passed all five final checkpoint jobs: https://github.com/mattsp1290/anchorfs/actions/runs/39057396357 .

Local environment: Rust 1.95.0; macOS development host; Docker Engine 29.2.1. Tests create disposable containers using the official multi-platform image postgres:17@sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3. Testcontainers' module fsync-off default is explicitly disabled; unsafe-setting tests override it intentionally. No existing infrastructure data is touched. Docker Engine is a host runtime rather than a container image; no engine image digest is applicable.

Observed work-package checks: core 7 tests; persistence 14 tests; backend/mount 9 macOS tests; service 2 middleware/policy + 6 binary + 5 OpenAPI tests; conformance 13 router + 3 design + 2 provenance tests. The full local workspace suite passed 61 tests, with fmt, clippy, build, cargo-deny, no-run compilation and Docker-free platform tests passing. Local tests ran on macOS; final GitHub CI independently passed Linux tests/lint/deny, native Linux arm64 build, and the macOS build/lint/no-run/Docker-free test job. No kernel FUSE or provider runtime acceptance is inferred.

## Fresh architecture review

A fresh approved-context architecture reviewer inspected all persistent columns, B1–B13, locks/fencing, API and dependency/source records at base revision 7281b665fed61076613e522241f3672f4d6b35a9. Initial technical verdict REQUEST_CHANGES; provenance no blocking provenance findings. Accepted findings: missing future success responses, missing session/workflow wire authority, and unresolved receipt expiry/replay lifecycle.

All were addressed and the same reviewer performed an explicitly labeled recheck (not a new independent initial review). Final reviewed tracked diff SHA-256 16035da88afb9646ce4e18924f1ed18afebf85911b1f042603b9a94fc4e8ebd9 against that base, excluding this subsequent evidence-record update. Technical APPROVE; provenance no blocking provenance findings. Independently verified 36 tests and all seven baseline hashes. Report artifacts remain outside commits under .agents/reviews/. Review is procedural evidence, not legal clearance or runtime storage acceptance.

The gauntlet's two independent standard reviewers initially requested changes and separately returned APPROVE on recheck, with no blocking provenance findings. Additional fixes define root selection/admin receipt inspection, make credential identity/scope immutable to close a reproduced concurrent issuer-scope race, strengthen provenance version validation, and reject invalid UTF-8 query decoding. Permanent operation tombstones preserve live FKs and replay keys; receipt_expire atomically revokes the associated session. Future success schemas and runtime shape validation are verified; expiry/publication/authentication execution remains Stage 1. Secret/token receipt inspection is explicitly excluded, with lost-grant recovery limits recorded in contracts.md.

The generic fixer security guard was overridden by the repository owner's explicit “I approve” for reviewed fixes. This approval authorizes implementation fixes; it is not the final architecture/timer sign-off below. The requested opus review model is unavailable on this platform: independent reviewers used the available Codex model, a disclosed deviation from that model-specific skill instruction.

## Review gauntlet

Standard review directory: `reviews/feat-stage0-contract-freeze-2026-10-17-135401-7281b665fed6`. Two independent reviewers initially REQUEST_CHANGES, then APPROVE on labeled rechecks. Canonical fixer summary: fixed 6, skipped 0, needs-manual 0 after the owner's explicit override. Standard checkpoint commit 05bd5abd89c5f78566e4189e54429b68b86e74cb pushed and verified equal to the remote branch.

Thermonuclear review directory: `reviews/feat-stage0-contract-freeze-thermo-2026-10-17-140911-05bd5abd89c5`. Upstream cursor/plugins main resolved to 101b274dd991f694e1f7024be2157e5f3adf07de at stage entry. The complete [pinned rubric](https://raw.githubusercontent.com/cursor/plugins/101b274dd991f694e1f7024be2157e5f3adf07de/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md) was fetched, frontmatter-validated and read as untrusted quality criteria only. A fresh reviewer received approved code/spec/design/dependency inputs, with earlier review conclusions withheld from this evidence record; origin/design portions were retained. The full branch evidence was gathered separately. No excluded research or competitor material was consumed.

Initial thermonuclear verdict REQUEST_CHANGES: one Important route-policy finding and one enum suggestion. The canonical fixer grouped individual route registrations by shared body/deadline policy, removed URL-suffix inference, preserved the prior shared PUT/DELETE files behavior, and added finite response enums with core conversion. No dependency or scope change. New tests exercise actual route groups with distinct deadlines and verify the enum vocabulary. The same reviewer independently reran all 61 tests and compared the YAML semantically, then returned APPROVE with no remaining findings and no blocking provenance findings. Exact reviewed fix diff SHA-256 4bda84af117c05b0a5ca37879915cff9dfe9b78a2a1914b84ce64bc8f59164ec against 05bd5ab; this subsequent evidence-record prose is outside that identity. Fixer summary: fixed 2, skipped 0, needs-manual 0.

All local applicable gates passed: fmt, full Clippy with warnings denied, build, 61 workspace tests, no-run test compilation, Docker-free core/backend/mount tests, cargo-deny license/bans/advisory/source gates and whitespace checks. Separate review artifacts are ignored by the local review conventions and are not committed. The two checkpoints are separate commits/pushes; the final checkpoint identity is recorded by branch history and remote verification rather than a self-referential hash in this file.

## Repository owner sign-off

Accepted on 2026-10-17 by the repository owner through the explicit response **“Approve architecture and timer defaults”** to the final sign-off question. The question identified docs/design/contracts.md, the completed two-stage gauntlet, all 61 passing local tests and all five green final-code CI jobs. This acceptance covers the Stage 0 architecture record and U2 timer defaults at reviewed/pushed code checkpoint bcb09dcdb72852a43b15b90828c2fa298513d965, including the documented later-stage limitations. It is separate from the earlier approval to apply reviewed fixes. The implementation agent records the owner's response; it does not substitute its own sign-off or claim legal clearance.

The Stage 0 sign-off prerequisite for later Stage 1 planning is satisfied. No Stage 1 implementation or deployment is authorized or claimed by this record. Current revision and checks are available from branch history and the draft PR.
