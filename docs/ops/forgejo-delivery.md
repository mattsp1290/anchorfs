# Forgejo delivery of `main` to Zot

status: prepared

This document describes the delivery path from GitHub `main` to the apartment
Zot registry and what each run leaves behind. It is an operations record, not
evidence that a run has happened; the `status:` line above changes to
`status: activated <run URL>` only when a mirrored main push has produced a
recorded `publication.json`.

```text
GitHub mattsp1290/anchorfs main ──Ferry (exact mirror)──▶ Forgejo mattsp1290/anchorfs main
                                                                  │ push event (refs/heads/main only)
                                                                  ▼
                   runner birb-optin-lunus-macbook-pro-linux-arm64, job container
                   10.0.0.106:30001/birbhomes/ci-rust-oci@sha256:4ac11af555a6be2b8ff7b74ae78b9c1a5060c841634d855111b0f6cc892fdbac
                   checks ──▶ build-export ──▶ publish
                                                  │ digests only
                                                  ▼
                   Zot http://10.0.0.106:30001  anchorfs/service, anchorfs/mount, charts/anchorfs
```

## Trust boundary

Ferry mirrors every GitHub branch (`refs/heads/*`) and tag (`refs/tags/*`). Any workflow file on any mirrored branch
executes on the same runner with this repository's Forgejo Actions secrets, and
Zot accepts anonymous writes from every reachable client. Therefore:

- The only secret installed on the Forgejo repository is
  `BIRBHOMES_SOURCE_READ_TOKEN`, scoped to read the birbhomes Forgejo mirror.
  No other secret is approved.
- Only the digests listed in `publication.json` of a recorded `refs/heads/main`
  run are trusted. A digest present in Zot is not evidence by itself.
- Ferry does not mirror pull request refs (its README at commit `fafa1ab` lists
  them as not mirrored), so fork pull requests never reach the Forgejo runner;
  only branches pushed by GitHub collaborators of `mattsp1290/anchorfs` do.
- "Only main publishes" is a property of `.forgejo/workflows/deploy.yml`
  (trigger plus per-job `if:` and ref checks) and of the birbhomes policy
  (`branch: main`), not a security boundary. Anyone who can push a branch to
  the GitHub repository can run arbitrary workflow code on the runner.

## Workflow keys not relied upon

The workflow declares `permissions: contents: read` and a `concurrency:` group.
Whether this Forgejo server and runner honor either key is unverified, so
neither is a control: the automatic token scope may be the server default, and
two quick main pushes may run `publish` concurrently (harmless, because writes
are digest-only and untagged). Confirm support during the first real run and
record the result here before treating either key as effective.

## Jobs

All three jobs run in the digest-pinned CI container on the exact runner label.
Right after checkout each runs `scripts/ci/guard.sh <tools...>` (one script, so
the three jobs cannot drift): the tools exist, the host is linux/aarch64,
Python is 3.11 or newer, `git rev-parse HEAD == github.sha` (40 lowercase hex)
and `github.ref == refs/heads/main`. Nothing is built before it passes.

1. `checks` — `cargo fmt --check`, `clippy -D warnings`, `cargo build --locked`,
   container-free tests (`anchorfs-core`, `anchorfs-backend`, `anchorfs-client`,
   service unit + OpenAPI snapshot, conformance `provenance` and `chunking_props`,
   mount `cli`), exporter unit tests, `scripts/check-chart.sh` (lint and render
   checks under Helm 4.3.0 and 3.13.0), `git diff --exit-code Cargo.lock`. `cargo deny`
   stays in GitHub CI (not in the CI image).
2. `build-export` — `scripts/build-release-binaries.sh` (cargo-zigbuild, both
   GNU targets at glibc 2.36), `crane pull --insecure --format=oci` of the
   digest-pinned bases listed by `oci_export.py bases pull-refs` with an empty
   credential store, `helm-compat package --destination dist/chart` (the single
   archive there is passed to the exporter), `scripts/oci_export.py` → three OCI layouts,
   `build.json` (v1 receipt), `SHA256SUMS`, `export-evidence.json`. No registry
   write, no daemon. Uploads `oci.tar` + `oci.tar.sha256`.
3. `publish` — downloads and checksum-verifies the layouts, checks out the
   birbhomes mirror at the immutable helper commit
   `40ff258cc56aa67ba22d8661dd0d11babe2d4065` with the read-only token
   (`persist-credentials: false`), verifies the pin and the policy identity,
   refuses a downloaded `build.json` whose `source_sha` or `run_id` differs from
   this run's `github.sha`/`github.run_id`, and runs `python3 -I birbhomes/scripts/oci-publish.py` with
   `deploy/contracts/apps/anchorfs.json`. The publisher validates the whole
   OCI closure, uploads digest-addressed manifests only (no tags) and reads
   every blob back before writing the receipt. Uploads `publication.json`.

Recorded deviation from the contract's wording: the publisher imports only the
Python standard library, so the job uses the image interpreter directly instead
of a venv plus `pip install`; this avoids PyPI egress the runner has no reason
to have. PyYAML and Helm are needed by the promoter, which is out of scope.

## Reading `publication.json`

```json
{"artifacts": {"service": {"repository": "10.0.0.106:30001/anchorfs/service", "digest": "sha256:…", "platforms": {"linux/amd64": "sha256:…", "linux/arm64": "sha256:…"}},
               "mount":   {"repository": "10.0.0.106:30001/anchorfs/mount",   "digest": "sha256:…", "platforms": {…}},
               "chart":   {"repository": "10.0.0.106:30001/charts/anchorfs",  "digest": "sha256:…", "platforms": {}}},
 "artifact_set_digest": "sha256:…", "source_sha": "…", "run_id": "…"}
```

`scripts/cluster/install.sh <publication.json>` consumes exactly these fields.
Image `digest` values are index digests (both platforms); the chart `digest` is
the manifest digest.

## Readback and retention (G12)

From the control host (no `crane` there), verify each digest with the
Distribution API immediately after the run and again at least two hours later:

```sh
for ref in anchorfs/service@sha256:… anchorfs/mount@sha256:… charts/anchorfs@sha256:…; do
  repo=${ref%@*}; digest=${ref#*@}
  curl -fsSI -H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json' \
    "http://10.0.0.106:30001/v2/$repo/manifests/$digest" | head -1
done
```

The publisher writes untagged manifests. On the running zot v2.1.3 with no
`retention` section in its configuration, untagged manifests are kept (the
untagged CI image has been served for weeks under hourly garbage collection).
A zot upgrade or a `retention.policies` entry matching these repositories would
change that, so the T+2 h readback is required evidence for every run and a
failed readback stops any install and reopens the birbhomes request.

## Rolling the helper pin

`BIRBHOMES_HELPER_COMMIT` (workflow `env` and the `ref:` of the birbhomes
checkout step) must be a 40-character commit present on the birbhomes Forgejo
mirror and recorded in `docs/provenance/dependencies.md`. Change both places
together; the provenance test requires 40 hex characters and that the two are equal.

## What a failed run leaves behind

A failed `publish` can leave unreferenced blobs or manifests in Zot but never a
success receipt. Rerunning is safe: it publishes a new, independently trusted
`publication.json`. Digests are not expected to match an earlier run (the chart
archive embeds packaging time and binary reproducibility is not established),
so never compare a rerun's digests with an earlier receipt as a tamper check.
A failed `build-export` leaves nothing outside the runner's job directory.

## Merging and `source_sha` in `docker/bases.json`

Merge pull requests that change `docker/bases.json` with a true merge commit
(not squash or rebase). The `source_sha` values recorded there (currently
`a430d02ab19c33061ffd558867133c68fb3ab8d4`, a feature-branch commit) stay
reachable from `main` only if the original commits are preserved. If a
squash or rebase happens anyway, re-record `source_sha` as the `main` commit
whose Dockerfile base stages are byte-identical (the `stage_sha256` check proves
that) and note the substitution in the Stage 3b review record.

## Identities

- Runner label: `birb-optin-lunus-macbook-pro-linux-arm64`.
- CI container: `10.0.0.106:30001/birbhomes/ci-rust-oci@sha256:4ac11af555a6be2b8ff7b74ae78b9c1a5060c841634d855111b0f6cc892fdbac`.
- Actions: `actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683` (v4.2.2),
  `code.forgejo.org/actions/upload-artifact@ff15f0306b3f739f7b6fd43fb5d26cd321bd4de5` (v3.2.1),
  `code.forgejo.org/actions/download-artifact@a9bc5e6ef2cb54c177f32aa5726adaa15e7e2d59` (v3.1.0, same artifact-API generation as the upload action proven on this runner).
- Chart: `charts/anchorfs` (the version lives only in `Chart.yaml`). The workflow
  packages into `dist/chart/` and passes the one archive found there to the exporter.
- Hub request: `birbhomes-r-80he` (platform side), `anchorfs-r-mx8o` (application side).
