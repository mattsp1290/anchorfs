# Dependency provenance

Source category: general-purpose libraries from the public crates.io registry. Metadata below was checked using `cargo info --registry crates-io` on 2026-10-17, before incorporation. Versions are exact workspace pins. Cargo.lock records the complete resolution. No competitor implementation is an input. AWS SDK crates are official platform adapters permitted by spec 02. New AnchorFS code is independently authored from docs/spec and the Stage 0 plan.

## Crates

| Crate | Version | License | Repository | Role |
|---|---|---|---|---|
| async-trait | 0.1.92 | MIT OR Apache-2.0 | https://github.com/dtolnay/async-trait | object-safe asynchronous backend |
| aws-credential-types | 1.3.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | lazy credential provider future for synchronous network-free construction |
| aws-smithy-http-client | 1.5.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | explicit CA trust bundle with TLS verification |
| aws-config | 1.12.0 | Apache-2.0 | https://github.com/smithy-lang/smithy-rs | official AWS configuration |
| aws-sdk-s3 | 1.152.0 | Apache-2.0 | https://github.com/awslabs/aws-sdk-rust | official S3 client; raw observer in conformance tests |
| axum | 0.8.9 | MIT | https://github.com/tokio-rs/axum | HTTP server |
| bytes | 1.12.1 | MIT | https://github.com/tokio-rs/bytes | binary buffers |
| cargo_metadata | 0.23.1 | MIT | https://github.com/oli-obk/cargo_metadata | workspace edge verification |
| clap | 4.6.7 | MIT OR Apache-2.0 | https://github.com/clap-rs/clap | CLI/environment configuration |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/futures-rs | failing body stream for the conformance fault proxy (dev-dependency) |
| fuser | 0.18.0 | MIT | https://github.com/cberner/fuser | Linux FUSE boundary |
| nix | 0.31.3 | MIT | https://github.com/nix-rust/nix | test process signals |
| rand | 0.9.5 | MIT OR Apache-2.0 | https://github.com/rust-random/rand | random secrets; conformance fixtures |
| reqwest | 0.12.28 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest | binary HTTP tests and the conformance fault proxy |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde | serialization |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json | JSON |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes | SHA-256 integrity |
| sqlx | 0.9.0 | MIT OR Apache-2.0 | https://github.com/launchbadge/sqlx | PostgreSQL and migrations |
| testcontainers-modules | 0.15.0 | MIT | https://github.com/testcontainers/testcontainers-rs-modules-community | PostgreSQL test image module |
| testcontainers | 0.27.3 | MIT OR Apache-2.0 | https://github.com/testcontainers/testcontainers-rs | disposable test infrastructure |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror | typed errors |
| time | 0.3.55 | MIT OR Apache-2.0 | https://github.com/time-rs/time | timestamps |
| tokio | 1.53.3 | MIT | https://github.com/tokio-rs/tokio | async runtime |
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
