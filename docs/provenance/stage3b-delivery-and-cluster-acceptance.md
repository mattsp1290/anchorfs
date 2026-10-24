# Stage 3b executable acceptance map

This map separates tests present from executed evidence. A row changes from
pending only with real evidence; pending is never replaced by inference. No
Flux promotion, HA, Spaces, sandbox, legal clearance or published/deployed
artifact is claimed.

| Criterion | Executable evidence | Executed status |
|---|---|---|
| 1 mirror and trigger | Forgejo run on a mirrored main push; negative non-main push | pending (external request and first mirrored push outstanding) |
| 2 runner checks | `checks` job log | pending |
| 3 daemonless export | `build-export` job, `export-evidence.json`, exporter unit tests (`scripts/tests/test_oci_export.py`), G1 local trial | unit tests passed on the workstation (13); G1 **passed** 2026-10-22 inside `ci-rust-oci@sha256:4ac11af…` (linux/arm64, uid 10001, rustc 1.95.0, cargo-zigbuild 0.23.4, zig 0.16.0): both targets built in 6m21s, arm64 binaries executed; G9 **passed**: export + shared publisher at `40ff258…` against a throwaway loopback registry returned `published`, all three digests read back, the chart fetched by digest renders, the published arm64 service image ran as uid 10001; Forgejo run pending |
| 4 publication | `publication.json`; Distribution-API readback (`curl`) from the control host | pending |
| 4b retention (G12) | the same three digests read back at T+0 and T+2 h on the running zot configuration | pending |
| 5 chart install and Helm test | `scripts/cluster/install.sh` transcript, `helm test`, `probe` binary tests | probe binary tests, chart lint and `check-chart.sh` (Helm 4.3.0) passed locally; install and `helm test` pending |
| 6 two-node proof | `cluster` test output and `cluster-evidence.json` | pending (test authored separately; never run) |
| 7 records | provenance tests, `dependencies.md`, this map | provenance tests passed locally (16 after the standard review); standard and thermonuclear code-review checkpoints complete 2026-10-23 (see the review record); GitHub CI green on the first checkpoint commit |
| image packaging regression | `ci/image-smoke.sh` on the restructured Dockerfile | local linux/arm64 passed; GitHub `image-amd64` and `image-arm64` pending |

See [review evidence](stage3b-delivery-and-cluster-review.md) for commands,
deviations, plan review stages and limits. Layers with executed evidence today:
workstation only. GitHub CI run URL, Forgejo run URL and job IDs, and cluster
run date, host and kernels: none yet.
