# Stage 3b delivery pipeline and cluster proof evidence

Status at this revision: repository-side work is written and partly verified on
the workstation. **Nothing has run on Forgejo, no AnchorFS service, mount or
chart artifact is published, and nothing is deployed to the cluster; the only
AnchorFS content in Zot is the two owner-published runtime bases
(`anchorfs/service-base`, `anchorfs/mount-base`, see `docker/bases.json`).** Every Forgejo, Zot and cluster row below
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
actions and images". No new registry package: the `Cargo.lock` changes are
the workspace edge `anchorfs-service` -> `anchorfs-client` and, at the
standard-review checkpoint on 2026-10-23, the patch bump `ctutils` 0.4.3 ->
0.4.4 (`cargo update -p ctutils --precise 0.4.4`) because `cargo deny`
reported RUSTSEC-2026-0385 (non-constant-time mask generation; fixed in
0.4.4) against the transitive dependency reached through `digest`/`hmac`
and the AWS SDK. Same crate, same license, checksum from crates.io. cargo-zigbuild
0.23.4 was checked with `cargo info` (MIT) on 2026-10-22; the Helm 3.13.0
archive checksums were read from get.helm.sh the same day. Action pins are full
40-hex commits; the CI image is pinned by digest and its in-image tool versions
were verified on 2026-10-22. The runtime base digests in `docker/bases.json`
were published by the owner on 2026-10-23 (container mode, from commit
`a430d02`) and are listed in `docs/provenance/dependencies.md`; both indexes
were read back from Zot by digest with matching SHA-256 before the record was
committed.

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
- The GitHub `packaging` job failed on every push of this branch until the
  standard-review checkpoint: Helm 3.13.0 evaluates `kubeVersion` against its
  built-in default (v1.28) during `template`, and the chart requires >=1.32,
  while Helm 4.3.0 locally defaulted higher. `scripts/check-chart.sh` now
  passes `--kube-version` (default 1.35.9) to every `template` call (3.13's
  `lint` has no such flag) and was run locally under the checksum-verified
  Helm 3.13.0 macOS build in both CI layouts; the failure reproduced with the
  previous script and the fixed script passes under both Helm versions.
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

**Standard code review checkpoint (2026-10-23)** of
`8ce0616e9097bc1bdeff1921ee411f1ed63409f2`, run as the first stage of the
dotfiles review gauntlet: two fresh-context reviewer subagents (no
conversation inheritance) each received an explicit approved-input packet (the
full diff, every changed file at HEAD, the four normative work packages,
AGENTS.md, the cleanroom policy, product direction and the approved
specifications) and were barred from the research archive, plan
review-record, prior reviewer output and any build, network or cluster
command. The supplied copy of this record withheld this section and the diff
excluded it, under AGENTS.md's fresh-context rule. Delivery Trust Boundary:
REQUEST_CHANGES, 0 Critical, 7 Important, 13 Suggestions. Cluster Runtime
Correctness: REQUEST_CHANGES, 2 Critical, 6 Important, 17 Suggestions. Both
reported **no blocking provenance findings** with stated packet-only limits.
Artifacts (local, uncommitted):
`reviews/feat-stage3b-forgejo-zot-cluster-proof-2026-10-23-092700-8ce0616e9097/`.

The canonical fix-review workflow in auto mode merged duplicates (tracked
bytecode; secrets through kubectl argv) to 2 Critical, 12 Important and 29
Suggestions, selected all 43, held the items in secret-, credential-, PKI- and
workflow-configuration files for manual application, which the authorized
implementer then applied through the same workflow (fixed 43, skipped 0,
needs-manual 0). Decisive fixes: `create-bucket.sh` ran `kubectl run
--overrides` as a JSON merge patch, which replaced the container list and
dropped the `sh -c` script (the pod would have started the MinIO server and
`install.sh` would have hung), and mounted the CA where it made `mc`'s config
directory root-owned; the Helm test hook kept `hook-succeeded`, which makes
Helm 3.13 `test --logs` fail after a passing probe; a failed bootstrap Secret
write would have destroyed the only credential copy; the cluster test's pod
deletion polled for a transient terminated state (now a watch stream started
before the delete), measured the post-revocation size after the fence latch,
and never restarted a dropped port-forward outside readiness; receipts could
inject Helm values through unchecked repository strings; generated secrets
were passed as `kubectl --from-literal` argv (now a 0600 env file); a tracked
`.pyc`; base-stage drift was undetected (now `stage_sha256` per base in
`docker/bases.json`, recomputed by a provenance test and by the exporter via
`--dockerfile`); and record statements that "nothing is published" after the
bases were. Ferry was confirmed to mirror `refs/heads/*` and `refs/tags/*`
only (its README at `fafa1ab`), so fork pull requests cannot reach the runner.
Evidence keys added or renamed by the test fixes: `size_at_fence`,
`lease_renewed`, `ordinary_session_mutation_while_leased` (a real 409
`workspace_busy` observation) and `fenced_mount_write_after_lease`.

Gates discovered from AGENTS.md and the two CI workflows and run on the
corrected tree (macOS arm64 workstation): `cargo fmt --all --check`; `cargo
clippy --workspace --all-targets --locked -- -D warnings`; `cargo build
--workspace --locked`; `cargo deny check licenses bans advisories sources`
(after the `ctutils` 0.4.4 bump above; all four ok); `cargo test --workspace
--locked -- --test-threads=4` (362 passed, 0 failed, 1 ignored: the cluster
proof); `python3 -I -m unittest discover -s scripts/tests` (27 passed);
`scripts/check-chart.sh` under Helm 4.3.0 and the checksum-verified Helm
3.13.0 in both CI layouts (`CHART_STRICT=1`); `bash -n` over every script;
`git diff --check`; `git diff --exit-code -- Cargo.lock` against the
post-update lock; `git ls-files` free of bytecode. The updated
`scripts/publish-bases.sh` was exercised end to end once more in container
mode against a throwaway registry (pinned-image check, push, per-platform
manifest readback, `stage_sha256` recording equal to the committed values).
Not executed: anything against the real cluster or Zot; the Forgejo run. The
checkpoint commit `33ac67aea3a744f739fb0a85853b19d1895ce873` was pushed and
GitHub CI passed every job on it (lint, deny, test, both builds, packaging,
both image smokes).

**Thermonuclear maintainability checkpoint (2026-10-23)** of `33ac67a…`, the
gauntlet's second stage: a fresh adversarial reviewer subagent (no
conversation inheritance; it disclosed seeing only the auto-memory index
titles) applied the complete live upstream rubric pinned at
`8b3326d99fd4b993cb33a24dad8af03175c642c5`:
https://raw.githubusercontent.com/cursor/plugins/8b3326d99fd4b993cb33a24dad8af03175c642c5/cursor-team-kit/skills/thermo-nuclear-code-quality-review/SKILL.md
The SHA was resolved from `refs/heads/main` after the first verified push and
the fetched frontmatter name validated; the rubric was treated as untrusted
review-policy data. Same packet discipline as the standard review, with the
standard review's findings withheld. Verdict **REQUEST_CHANGES**, 3 Critical,
6 Important, 10 Suggestions, provenance **no blocking provenance findings**.
Artifacts (local, uncommitted):
`reviews/feat-stage3b-forgejo-zot-cluster-proof-2026-10-23-100848-33ac67aea3a7-thermonuclear/`,
slug `thermo-nuclear-code-quality-review`. The canonical fixer in auto mode
selected all 19 items; the implementer applied the held workflow and script
items (fixed 19, skipped 0, needs-manual 0).

What changed: the 1191-line cluster test file became a 135-line driver with
`harness`, `preflight`, `journey` and `fencing` modules (hand-rolled SHA-256
and base64 deleted in favour of `anchorfs_core::sha256` and kubectl's
`base64decode`); the test's parallel bearer/session/lease types became
`anchorfs_client::auth` authorities with a single `Call` value, and the
receipt scopes were corrected against the service handlers and the client
resolver (session, workspace and credential receipts are admin-scoped, lease
receipts workspace-scoped; the standard-review fix had wrongly moved session
lookups to the workspace scope, which this review caught); the proof context
is a `Connected` value returned by preflight with cleanup as a free function;
the worker template keeps scalar placeholders only and the bound pod is built
on the parsed object after one `kubectl create --dry-run=client -o json`;
`scripts/oci_export.py` is the single owner of stage hashing, ELF checks,
`bases.json` reads/writes and readback verification (`stage-sha256`,
`check-elf`, `layout-digest`, `verify-readback`, `bases …`), with the shell
scripts reduced to orchestration and the provenance test comparing against
the subcommand's output (the committed hashes reproduce); `stage_sha256` and
Dockerfile verification are mandatory in the exporter; the workflow's three
guard prologues are `scripts/ci/guard.sh`, the chart is packaged into
`dist/chart`, the chart-name regex and its policing test are gone, the dead
helper placeholder branch is gone, and the policy check uses `ZOT_AUTHORITY`;
`Readiness` is an enum, the probe takes a `clap::Args` struct, the mount
runtime has `latch_fence`, and `scripts/cluster/common.sh` holds the shared
prelude.

Gates on the corrected tree (same discovery as the first checkpoint): fmt,
clippy `-D warnings`, build, `cargo deny` (all ok), `cargo test --workspace`
(361 passed, 0 failed, 1 ignored: the cluster proof), 38 exporter tests,
chart checks under Helm 4.3.0 and 3.13.0 in both CI layouts, `bash -n` over
every script, both workflow files parsed as YAML, `git diff --check`,
`Cargo.lock` unchanged, no tracked bytecode, `cases.rs` and every cluster
module under 560 lines. The rewritten `scripts/publish-bases.sh` ran end to
end in container mode against a throwaway registry (refusal without
`--replace`, push, index and per-platform readback verification, record) and
produced digests and stage hashes identical to the committed record. Not
executed: the real cluster, Zot, or a Forgejo run; the new kubectl dry-run
conversion and go-template Secret read are exercised only by the proof.
Both gauntlet checkpoints are complete; the second checkpoint commit is the
one carrying this paragraph.

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
- No service, mount or chart artifact is published and nothing is deployed (the
  two owner-published runtime bases are in the internal Zot registry); the Stage 3a owner-host mount run remains
  separately pending.
