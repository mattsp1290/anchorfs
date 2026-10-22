# Dependency provenance

Source category: general-purpose libraries from the public crates.io registry. Metadata below was checked using `cargo info --registry crates-io` on 2026-10-17, before incorporation. Versions are exact workspace pins. Cargo.lock records the complete resolution. No competitor implementation is an input. AWS SDK crates are official platform adapters permitted by spec 02. New AnchorFS code is independently authored from docs/spec and the Stage 0 plan.

## Crates

| Crate | Version | License | Repository | Role |
|---|---|---|---|---|
| async-trait | 0.1.92 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait | object-safe asynchronous backend |
| aws-credential-types | 1.3.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | lazy credential provider future for synchronous network-free construction |
| aws-smithy-http-client | 1.5.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | explicit CA trust bundle with TLS verification |
| aws-config | 1.12.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | official AWS configuration |
| aws-sdk-s3 | 1.152.0 | Apache-2.0 | https://github.com/awslabs/aws-sdk-rust | official S3 client; raw observer in conformance tests; mount fixture bucket setup |
| axum | 0.8.9 | MIT | https://github.com/tokio-rs/axum | HTTP server |
| bytes | 1.12.1 | MIT | https://github.com/tokio-rs/bytes | binary buffers |
| cargo_metadata | 0.23.1 | MIT | https://github.com/oli-obk/cargo_metadata | workspace edge verification |
| clap | 4.6.7 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap | CLI/environment configuration |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs | bounded body streaming in anchorfs-service and failing streams in the conformance fault proxy |
| fuser | 0.18.0 | MIT | https://github.com/cberner/fuser | Linux FUSE boundary |
| http-body-util | 0.1.5 | MIT | https://github.com/hyperium/http-body | typed length-limit error classification for streamed replacement; reviewed 2026-10-20 |
| libc | 0.2.191 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc | Linux mount open/mode constants and raw test syscalls |
| nix | 0.31.3 | MIT | https://github.com/nix-rust/nix | test process signals |
| rand | 0.9.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand | random secrets; conformance fixtures |
| reqwest | 0.12.28 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest | native-API client (`anchorfs-client`), binary HTTP tests and the conformance fault proxy |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde | serialization |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json | JSON and persistence receipt payloads |
| proptest | 1.12.0 | MIT OR Apache-2.0 | https://github.com/proptest-rs/proptest | namespace model/property tests (dev-dependency) |
| hmac | 0.13.0 | MIT OR Apache-2.0 | https://github.com/RustCrypto/MACs | list-cursor message authentication |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes | SHA-256 integrity |
| sqlx | 0.9.0 | MIT OR Apache-2.0 | https://github.com/launchbadge/sqlx | PostgreSQL, migrations and JSON receipt payloads |
| testcontainers-modules | 0.15.0 | MIT | https://github.com/testcontainers/testcontainers-rs-modules-community | PostgreSQL test image module |
| testcontainers | 0.27.3 | MIT OR Apache-2.0 | https://github.com/testcontainers/testcontainers-rs | disposable test infrastructure |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror | typed errors |
| time | 0.3.55 | MIT OR Apache-2.0 | https://github.com/time-rs/time | timestamps |
| tokio | 1.53.3 | MIT | https://github.com/tokio-rs/tokio | async runtime; process feature for the event-driven non-lazy unmount helper |
| toml | 1.1.8+spec-1.1.0 | MIT OR Apache-2.0 | https://github.com/toml-rs/toml | manifest verification |
| tower-http | 0.6.11 | MIT | https://github.com/tower-rs/tower-http | HTTP middleware |
| tower | 0.5.3 | MIT | https://github.com/tower-rs/tower | service composition and testing |
| tracing-subscriber | 0.3.23 | MIT | https://github.com/tokio-rs/tracing | logging |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing | structured events |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url | endpoint parsing |
| utoipa-axum | 0.3.0 | MIT OR Apache-2.0 | https://github.com/juhaku/utoipa | route/document registration |
| utoipa | 6.0.0 | MIT OR Apache-2.0 | https://github.com/juhaku/utoipa | OpenAPI annotations |
| uuid | 1.28.0 | Apache-2.0 OR MIT | https://github.com/uuid-rs/uuid | random identifiers; conformance fixtures |

## Tools

| Tool | Version | License | Repository | Role |
|---|---|---|---|---|
| cargo-deny | 0.20.2 | MIT OR Apache-2.0 | https://github.com/EmbarkStudios/cargo-deny | full dependency license, source, advisory gate |

## CI actions

| Action | Commit | License | Repository | Role |
|---|---|---|---|---|
| actions/checkout | 34e114876b0b11c390a56381ad16ebd13914f8d5 | MIT | https://github.com/actions/checkout | checkout v4.3.1 |
| actions/cache | 0057852bfaa89a56745cba8c7296529d2fc39830 | MIT | https://github.com/actions/cache | Cargo cache v4.3.0 |

Action tag SHAs were resolved with git ls-remote. License files are verified at those pinned revisions. Toolchain installation uses rustup directly; no toolchain action is incorporated.

## License and advisory policy

The allow list in deny.toml is MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0 and CDLA-Permissive-2.0. No advisory ignores are authorized.

A new direct dependency or CI action requires a row with version, license, repository and role before the commit that adds it. A transitive license outside the D13 allow list is a blocker, recorded with crate name and license under "Pending owner decision", and never resolved by widening deny.toml without the owner. A cargo deny advisory failure is resolved by upgrading; an ignore entry needs the owner's approval recorded with advisory ID and rationale.

## Pending owner decision

None recorded.

## WP5 SDK boundary additions

On 2026-10-17, `cargo info` verified aws-credential-types 1.3.0 and aws-smithy-http-client 1.5.0 as Apache-2.0 from the official Smithy repository. These were already resolved transitive dependencies; explicit direct declarations support a lazy chain provider without blocking an async runtime and custom CA configuration without disabling certificate verification. Registry dependency API sources were consulted for signatures only; AnchorFS adapter logic and synthetic tests are independently authored from approved spec 02/spec 04 and WP5. No competitor source or research archive was used. Enabling rustls-aws-lc and rt-tokio matches the existing official SDK TLS/runtime stack.

Conformance uses the already reviewed axum pin as a direct dev dependency to construct HTTP requests and decode router responses. This corrects the plan’s omitted HTTP test types; it adds no new package to the graph.

## Test images

| Image | Reference | Digest | Licenses | Role | Approval |
|---|---|---|---|---|---|
| postgres | 17 | sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3 | PostgreSQL License | disposable metadata database for tests | Stage 0 plan D2 |
| bitnamilegacy/minio | 2025.7.23-debian-12-r5 | sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20 | MinIO server AGPL-3.0; Bitnami container packaging Apache-2.0 (label org.opencontainers.image.source = github.com/bitnami/containers); Debian base packages under their respective licenses | disposable S3-compatible object store for tests; separate container process, never linked, vendored, mirrored or distributed by this repository | user decision E1, 2026-10-17, after the official minio/minio images were found unavailable |

Test images are pinned by digest and recorded before first use. Mirroring an image is redistribution and needs its own license review. Image origin and license observations above come from the approved Stage 1 plan's E1 provider diagnosis. The futures-core registry manifest was checked before adding its direct dependency. Fixtures and proxy logic are independently authored from the approved WP2 plan; no competitor material or research archive was consulted.

## Stage 1a feature additions

The SQLx json feature and persistence → serde_json edge use already reviewed,
locked packages. Service directly names SQLx for transaction types and error
mapping; conformance directly names SQLx, time and async-trait for disposable
metadata coordination, clock comparisons and the synthetic counting backend.
All sources/versions/licenses remain in the table above. No new crate or
workspace-member edge was introduced. `Cargo.lock` records the new direct edges.

Stage 1b: hmac 0.13.0 direct use checked with `cargo info --registry crates-io hmac@0.13.0` on 2026-10-18 (MSRV 1.85). Already present transitively; this adds no new package.

Stage 1b: proptest 1.12.0 metadata checked on 2026-10-18 with `cargo info --registry crates-io proptest@1.12.0` (MSRV 1.88). Default features disabled; std only, without fork/timeout. New transitive origins/licenses are recorded by Cargo.lock and verified by cargo-deny; planned additions are core_detect, rand_xorshift and unarray (MIT OR Apache-2.0).

## Stage 1c runtime edges — 2026-10-20

The existing futures-core 0.3.34 pin is now a runtime dependency of service body
streaming as well as conformance. http-body-util 0.1.5 was already transitively
locked; cargo info for that exact version and its local registry LICENSE were
checked before adding its workspace pin and direct service edge. Source is the
general-purpose hyperium HTTP body library, MIT; no competitor source or fixture
is involved. Its typed LengthLimitError distinguishes malformed declared body
length from transport/disconnect errors without matching error-message prose.
This is the plan's allowed reviewed dependency exception: neither package
version nor the resolved package set changes.

## Stage 2a security patch exception — 2026-10-20

Stage 2a adds no package or dependency edge. The owner explicitly approved
updating two existing transitive packages after the advisory gate failed:

| Package | Old version | New version | Origin | License |
|---|---|---|---|---|
| rustls-webpki | 0.103.15 | 0.103.16 | crates.io; official rustls/webpki repository | ISC |
| webpki-roots | 1.0.9 | 1.0.10 | crates.io; official rustls/webpki-roots repository | CDLA-Permissive-2.0 |

Exact-version `cargo info` source/license checks preceded incorporation. Both
licenses are already permitted by deny.toml. Cargo.lock changes only these two
versions and their registry checksums, addressing RUSTSEC-2026-0382 and
RUSTSEC-2026-0381. The owner-approved exception supersedes the plan's unchanged
lockfile constraint; imported specifications and the import manifest remain
unchanged. See [D-WL1 and verification](stage2a-implementation-amendments.md).
The post-update cargo-deny licenses/bans/advisories/sources gate passes.

## Stage 2b dependency role review (2026-10-21)

Before incorporating the runtime reqwest edge, `cargo info reqwest@0.12.28`
confirmed MIT OR Apache-2.0 and https://github.com/seanmonstar/reqwest.
The existing pinned general-purpose HTTP library is an approved input; no
competitor client or SDK was consulted. The shipped mount will embed the
existing `webpki-roots` trust roots through reqwest's rustls feature. The
`https` transport remains unexercised by this milestone's acceptance tests.

`cargo info libc@0.2.191` confirmed MIT OR Apache-2.0 and
https://github.com/rust-lang/libc before the planned Linux direct edge.
No new registry package is authorized by this milestone.

## Runtime tools for Stage 2b

`fusermount3` (fuse3 3.14.0 on the verification host; Ubuntu's fuse3 package
in CI) is an external helper process, never linked, vendored or distributed by
AnchorFS. libfuse is LGPL-2.1; the utility is GPL-2.0. `git` is GPL-2.0 and is
executed only as a synthetic smoke workload. These are permitted platform and
test tools from the reviewed Stage 2b plan; no competitor dependency is added.

The Linux mount test target reuses the repository's MinIO fixture source and
API response-loss relay by path. The new fixture, syscall tests and workloads
are independently authored from the plan and the existing service contract.
All added dev-dependencies were already pinned and licensed workspace inputs;
only workspace crate/dependency edges change in Cargo.lock.

Stage 2b structural hardening enables Tokio's existing `process` feature for the
non-lazy helper. The package version, source and license remain unchanged; no
registry package is added. RFC3339 serde helpers move from the independently
authored client wire module into core for the shared metadata-update type.
