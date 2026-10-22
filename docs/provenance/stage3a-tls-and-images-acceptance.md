# Stage 3a executable acceptance map

This map distinguishes tests present from executed acceptance. No cluster,
publication, cross-node, performance or legal clearance claim is made.

| Criterion | Executable evidence | Executed status |
|---|---|---|
| 1 TLS listener/readiness | binary `tls_tests::serve_tls_durable_non_loopback_reports_ready`, `tls_tests::serve_tls_development_ready_with_minio` | macOS disposable PostgreSQL/MinIO: passed |
| 2 TLS 1.3 / HTTP/1.1 | binary `tls_tests::serve_tls_rejects_plaintext_and_tls12`; unit `tls::tests::server_config_sets_http1_alpn_and_rejects_bad_input`; server `.http1_only()` | binary and unit passed on macOS; explicit HTTP/2 refusal also passed |
| 3 fail closed | binary `tls_tests::serve_tls_config_errors`, `serve_refuses_non_loopback_plaintext_durable` | all 17 final binary tests passed on macOS |
| 4 client verifies server | conformance `https_endpoint_verifies_server_certificate` (createSession, stat, mkdir, open, write, fsync, read, close; wrong CA leaves sessions/receipts unchanged) | passed on macOS |
| 5 Linux https mount | Linux `tls::https_mount_with_ca_bundle_round_trips`, `tls::https_mount_without_trusted_ca_exits_before_mounting` | hosted Linux 7.0.0-1017-azure: both passed; separate owner-host run pending |
| 6 native images | CI `image-amd64`, `image-arm64`: both targets and packaging smoke, including explicit architecture assertions for both images | local arm64 and native CI amd64/arm64 targets + smoke passed (run 39803347993) |
| 7 provenance | conformance `dependency_record_matches_workspace`, `base_image_record_matches_dockerfiles`, `builder_image_matches_toolchain`; cargo-deny | cargo-deny and all five provenance tests passed |
| 8 records/regression | Stage 3a records, contracts, dependency/ops/readme updates; OpenAPI snapshot, design coverage, imported hashes | hashes, OpenAPI, design coverage and 341 workspace tests passed; standard fixes applied; fresh thermonuclear APPROVE/zero findings; both local checkpoint gates and all seven hosted jobs passed; separate owner-host gate pending |

See [review evidence](stage3a-tls-and-images-review.md) for commands, revisions,
review checkpoints, run identifiers and limits. Kernel tests run the host binary;
image smoke does not prove containerized mounting.

The named hosted run covers PR head `e609b0211bece4eb9bf56b75b28c48525a7c1795`
(374 Linux tests, 31 kernel tests). Later evidence-only commits have their own
checks linked from [PR #8](https://github.com/mattsp1290/anchorfs/pull/8).
Full milestone acceptance remains pending the owner-host serial mount/full-suite
run and cleanup observations; it is not inferred from the hosted kernel run.
