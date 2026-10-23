# Stage 3b delivery pipeline and cluster proof evidence

Status at this revision: repository-side work is written and partly verified on
the workstation. **Nothing has run on Forgejo, nothing is published to Zot and
nothing is deployed to the cluster.** Every Forgejo, Zot and cluster row below
is pending and is updated only with real evidence. See the
[acceptance map](stage3b-delivery-and-cluster-acceptance.md).

## Approved inputs and independent design

Implementation branches from the Stage 3a tree (`origin/main`
`906ed4a497c450fe0e178525c5e0028402df6f55`) on
`feat/stage3b-forgejo-zot-cluster-proof`. The plan is
`.agents/plans/stage3b-forgejo-zot-cluster-proof/` (local, gitignored); its
input identities are recorded in `00-overview.md`. Imported specifications and
the import manifest are unchanged.

| Work | Approved requirement / decision | Origin |
|---|---|---|
| Runtime base split, `docker/bases.json` | spec 04 amd64/arm64; plan D2, D16 | independently authored Dockerfile restructure on the already-approved Debian base |
| Release binaries (`cargo zigbuild`) | plan D2, G1 | independently authored script using the published cargo-zigbuild CLI |
| OCI exporter `scripts/oci_export.py`, unit tests | OCI image-spec and distribution-spec; plan D15 | independently authored, standard library only; image-spec names express the standard |
| Helm chart, worker-template ConfigMap | Kubernetes volumes and Helm registry documentation; plan D4, D6, D9, D13, D17 | independently authored templates from platform documentation |
| `anchorfs-service probe`, `anchorfs_client::readiness`, mount log events | spec 03 readiness route; plan D14, finding A-F3 | independently authored AnchorFS code |
| Forgejo workflow | owner repository delivery contract read as a local ecosystem fact; plan D1, D3, D12 | independently authored workflow; shared publisher consumed at a pin, never copied |
| Cluster scripts, PKI manifest | cert-manager SelfSigned to CA bootstrap documentation; plan D6, D18 | independently authored |
| Cluster proof test | spec 07 leases and fencing; plan D5, D18, D19 | authored in a separate work package; its evidence is recorded there |
| Records | cleanroom checklist; plan WP4 | repository execution evidence |

No competitor implementation, SDK, binary, trace or research archive was
consulted. The owner's birbhomes, ferry and borb repositories are the owner's
own infrastructure, read as local ecosystem facts to learn the delivery
contract; no code was copied from them. Fresh review context provides
procedural separation, not proof of legal clearance or erased historical
exposure.

## Dependency and tool provenance

See [dependency provenance](dependencies.md), section "Stage 3b delivery tools,
actions and images". No new registry package: the only `Cargo.lock` change is
the workspace edge `anchorfs-service` -> `anchorfs-client`. cargo-zigbuild
0.23.4 was checked with `cargo info` (MIT) on 2026-10-22; the Helm 3.13.0
archive checksums were read from get.helm.sh the same day. Action pins are full
40-hex commits; the CI image is pinned by digest and its in-image tool versions
were verified on 2026-10-22. The runtime base digests in `docker/bases.json`
are null: the owner has not yet run `scripts/publish-bases.sh`.

## Executed verification

Layer legend: W = macOS workstation (arm64), GH = GitHub CI, F = Forgejo run,
C = cluster.

| Layer | Check | Result |
|---|---|---|
| W | `scripts/tests/test_oci_export.py` exporter unit tests | passed |
| W | `helm lint` and `scripts/check-chart.sh` under Helm 4.3.0 | passed (not yet under the pinned 3.13.0) |
| W | `anchorfs-service` probe binary tests (Docker for disposable services) | passed |
| W | Docker `service` and `mount` targets plus `ci/image-smoke.sh` on linux/arm64 against the restructured Dockerfile | passed |
| W | G1: `scripts/build-release-binaries.sh` for both GNU targets inside the pinned CI image (`docker run --user 10001:10001`, image fetched with curl and loaded locally) | **passed** 2026-10-22: in-image rustc 1.95.0, cargo-zigbuild 0.23.4, zig 0.16.0, Python 3.12.15; `Finished release in 6m 21s`; four ELF binaries with the expected `e_machine`; arm64 `anchorfs-service --help` and `anchorfs-mount --print-profile` executed |
| W | G9: throwaway `crane registry serve` on loopback; base stages built with buildx for both platforms and pushed; `crane pull --format=oci` by digest; `helm package`; `scripts/oci_export.py`; `oci-publish.py` from a detached birbhomes worktree at `40ff258cc56aa67ba22d8661dd0d11babe2d4065` with the policy's registry URL pointed at the throwaway | **passed** 2026-10-22: publisher printed `{"status": "published", ...}`; `publication.json` lists service/mount indexes with amd64+arm64 platform digests and the chart manifest; HTTP 200 readback for all three; chart manifest and layer fetched by digest with matching SHA-256 and `helm template` rendered; the published arm64 service image, loaded locally, ran `--help`, `id` (uid 10001) and `openapi` |
| W | `cargo test -p anchorfs-conformance --test provenance --locked` (12 tests: seven new delivery-record tests plus the extended crate-edge test) | passed |
| GH | `packaging` job and the existing seven jobs for this branch | pending (no run yet) |
| F | any run of `.forgejo/workflows/deploy.yml` | pending; no Forgejo run URL or job IDs exist |
| C | any install, `helm test` or two-node proof | pending; no cluster run date, host or kernels exist |

Final gate commands for the whole package (fmt, clippy, workspace tests,
`cargo deny`) are recorded here when they are run on the finished branch.

## Corrections and deviations

- The shared publisher is run with the CI image interpreter and no virtual
  environment (plan finding A-F4); egress to crates.io is listed in the request
  to the owner.
- `MINIO_SKIP_CLIENT=yes` is set on the dev MinIO because the Bitnami wrapper's
  loopback health probe cannot trust the private CA; verified from the image's
  own scripts. Bucket creation is therefore a separate step
  (`scripts/cluster/create-bucket.sh`).
- `scripts/cluster/install.sh` installs without `--wait` and waits for the
  service deployment after the bucket exists, since the service cannot become
  ready before the bucket does.
- The macOS workstation blocks non-Apple binaries (crane, python) from reaching
  the LAN through Local Network privacy, so the G1 trial fetched the CI image
  with `curl`. The same restriction hit `scripts/publish-bases.sh` on the first
  owner run (crane: "no route to host"), after a first failure because the
  script omitted `-f docker/Dockerfile`. Both fixed: the script now takes
  `CRANE_IMAGE` and runs crane inside Docker, whose VM reaches the LAN; the
  full script was then exercised from a clean worktree against a throwaway
  registry container (see the workstation gates below).
- Helm lint and chart checks have run only under Helm 4.3.0 locally; the
  pinned 3.13.0 gate runs in the `packaging` job and is pending.
- Not applicable so far: the G5 single-job fallback, the G2 fallback image and
  any NetworkPolicy remedy have not been needed or tried.

## Workstation gates on the implementation (2026-10-22, macOS arm64)

Run on the complete branch before the first commit: `cargo fmt --all --check`;
`cargo clippy --workspace --all-targets --locked -- -D warnings`;
`cargo build --workspace --locked`; `cargo deny check licenses bans advisories
sources` (advisories, bans, licenses, sources all ok; no new registry package);
`cargo test --workspace --locked -- --test-threads=4` (355 passed, 0 failed,
1 ignored: the cluster proof, which requires `ANCHORFS_CLUSTER_KUBECONFIG`;
includes the Docker-backed TLS/probe binary tests and the disposable
PostgreSQL/MinIO suites); `git diff --check`; `python3 -I -m unittest discover
-s scripts/tests` (13 passed); `bash scripts/check-chart.sh` (Helm 4.3.0; the
3.13.0 path runs in the GitHub `packaging` job and on the runner). The Linux
kernel mount tests, including the two new log-event assertions, have zero
tests on macOS and run in hosted CI and on the Linux host.

Base publication script (2026-10-23, after the owner's two failed runs): the
complete `scripts/publish-bases.sh` ran unmodified with `CRANE_IMAGE` set to
the locally loaded CI image, from a clean detached worktree whose
`docker/bases.json` named a throwaway registry container
(`crane registry serve` on the Docker bridge; Docker here is a Colima VM, so
host paths are not mountable and the OCI tarball is streamed over stdin). It
built both bases for both platforms, pushed each index, read it back by digest
and wrote `docker/bases.json`; the index digests equalled those of the owner's
own buildx run (`sha256:e97ee3a6…` service-base, `sha256:bb27e012…`
mount-base), so the stages build reproducibly from cache. The worktree,
registry container and layouts were removed afterwards; nothing reached Zot.

## Independent reviews and delivery

Plan reviews actually completed (dispositions in `review-record.md` in the plan
directory): two independent fresh-context initial reviews (completeness:
15 findings; architecture and verification: 15 findings) and one fresh
adversarial review (11 findings). All three reported no blocking provenance
findings; every finding was dispositioned in the plan.

**Code review checkpoints have not happened.** No standard code review, no
adversarial code review and no fix-review pass has run on this implementation,
and no commit or pull request exists yet. Those stages, each with a fresh
context and an explicit approved-input packet, are required before delivery and
will be recorded here.

## Limits carried forward

- Delivery trust boundary: Ferry mirrors every GitHub branch and Zot is
  anonymous, so only digests listed in `publication.json` of a recorded `main`
  run are trusted.
- No Flux promotion, no HA (single-replica dev PostgreSQL and MinIO), no Spaces,
  no sandbox, no performance claim.
- Restart-only certificate reload applies to cert-manager renewals.
- The privileged mount sidecar is trusted infrastructure.
- Zot retention is verified by readback on v2.1.3, not configured, and is
  version specific.
- Nothing is published or deployed; the Stage 3a owner-host mount run remains
  separately pending.
