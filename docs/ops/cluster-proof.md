# Two-node cluster proof

status: prepared

This document is the owner procedure for the two-node k3s proof of an installed
AnchorFS release. It is an operations record, not evidence that a run has
happened. The run is blocked until the published image and chart digests exist
(external request G6) and the Zot readback in
[forgejo-delivery.md](forgejo-delivery.md) passes. Nothing here certifies a
sandbox, a production scheduler, or node-loss durability.

The proof drives `kubectl` from the control host with a Rust integration test
(`crates/anchorfs-conformance/tests/cluster.rs`). It creates only
namespaced objects in the `anchorfs` namespace, each labelled
`anchorfs.proof/run=<uuid>`, and never touches cluster-scoped objects or other
namespaces. It is never run in CI.

## Prerequisites

- Logged in on `infra-control` with `k3s kubectl`, `helm` 3.13.0 and the Rust
  toolchain; `KUBECONFIG=/etc/rancher/k3s/k3s.yaml`.
- Host tools: `jq`, `curl`, `openssl` (for `create-secrets.sh`) and `sha256sum`
  or `shasum`. `install.sh` exports `KUBECTL` so `create-bucket.sh` uses the
  same kubectl client.
- A `publication.json` receipt from a Forgejo run with the `anchorfs/service`,
  `anchorfs/mount` and `charts/anchorfs` digests.
- Both arm64 worker nodes and the amd64 service node `Ready`. Defaults:
  workers `k3s-worker-raspberrypi,k3s-worker-raspberrypi-2`, service node
  `infra-control-master`. Override with the environment variables below if a
  named node is not Ready (for example to use `-5`).

## Procedure

Run from a clean committed checkout. Sync tracked files only, as in
[linux-host-verification.md](linux-host-verification.md):

```sh
git status --porcelain
git rev-parse HEAD
git ls-files -z | rsync -0a --files-from=- . <host>:anchorfs-verify/
```

Then on the host, from `~/anchorfs-verify`:

```sh
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
scripts/cluster/prepare-namespace.sh
scripts/cluster/create-secrets.sh    # fails on a partial Secret set; --rotate replaces all
# install.sh checks the receipt's repositories against the registry host:port,
# prints source_sha, run_id and artifact_set_digest (compare them with the
# recorded main run, or set EXPECTED_SOURCE_SHA=<40 hex> to have it compare),
# installs the chart from a temp file named by its layer digest, then runs
# create-bucket.sh, helm test --logs and bootstrap-admin. If storing the
# bootstrap Secret fails it saves the credential mode 600 under
# ${XDG_RUNTIME_DIR:-$HOME}/anchorfs-bootstrap-admin.json and prints the path.
scripts/cluster/install.sh <publication.json>
ANCHORFS_CLUSTER_KUBECONFIG=/etc/rancher/k3s/k3s.yaml \
ANCHORFS_CLUSTER_EVIDENCE=$HOME/cluster-evidence.json \
  cargo test -p anchorfs-conformance --test cluster --locked -- \
  --ignored --test-threads=1 --nocapture
```

Environment variables of the test:

| Variable | Default | Meaning |
|---|---|---|
| `ANCHORFS_CLUSTER_KUBECONFIG` | unset | Required. Unset makes the test panic with `blocked: ANCHORFS_CLUSTER_KUBECONFIG unset`. |
| `ANCHORFS_CLUSTER_KUBECTL` | `k3s kubectl` | kubectl command, split on whitespace. |
| `ANCHORFS_CLUSTER_NAMESPACE` | `anchorfs` | Release namespace. |
| `ANCHORFS_CLUSTER_WORKER_NODES` | `k3s-worker-raspberrypi,k3s-worker-raspberrypi-2` | Two distinct arm64 nodes; the first hosts pods A, C, E, the second B, D. |
| `ANCHORFS_CLUSTER_SERVICE_NODE` | `infra-control-master` | amd64 node expected to run the service. |
| `ANCHORFS_CLUSTER_EVIDENCE` | `target/cluster-evidence.json` | Output path (relative to the crate directory under `cargo test`; use an absolute path). |

The test runs phases 0 to 7: preflight, authority setup, harness-writes /
worker-reads, replacement on the other node, idle then attach, revoke-first
fencing then a lease-bound successor, ephemeral root, teardown. Every wait is at
most 180 s and the whole test at most 25 minutes. A failure prints the failing
phase, deletes the run's pods and Secrets best-effort and leaves the evidence
file with `"status":"failed"`. A FUSE failure on a node reports
`blocked: node <name>: <stage>` with the setuid bit of `fusermount3`, root
propagation, both containers' mountinfo, the sidecar log and `describe pod`.

## What to record

Attach the sanitized evidence file (node, architecture, kernel, pod UID, image
ID, digests, exit codes, timestamps; no credentials) to
`docs/provenance/stage3b-delivery-and-cluster-acceptance.md` together with the
run date, the source commit SHA, the image and chart digests, `helm status` and
`helm test` output, and the node kernel versions. Review the file for secrets
before committing it.

Afterwards:

```sh
scripts/cluster/uninstall.sh [--delete-data] [--delete-pki]
```

The test itself keeps the release and the workspace. `uninstall.sh` removes the
proof worker pods and run-labelled Secrets, the kept `anchorfs-test-probe` pod
(the chart keeps it after a pass so `helm test --logs` can read it) and the
one-off bucket pods, and states what it leaves behind.

## Recovery: stale FUSE mounts

A mount sidecar that dies without unmounting leaves a FUSE mount on the node.
Symptoms: a proof pod stuck `Terminating`, a `/workspace` fuse entry for that
pod in the node's mountinfo, or `blocked: node <name>: pod stuck Terminating`
from the test. The usual cause is a worker-side process holding a cwd or file
descriptor under `/workspace` when the sidecar tried its non-lazy unmount
(`unmount refused`); the test's quiesce step checks for this before deleting.

1. Identify the pod's volume directory on that node:
   `/var/lib/kubelet/pods/<pod-uid>/volumes/kubernetes.io~empty-dir/workspace`.
2. Run a privileged one-off cleanup pod pinned to that node (host PID not
   required) that mounts `/var/lib/kubelet/pods` with `Bidirectional`
   propagation and runs `fusermount3 -uz <that path>`. Lazy unmount is
   deliberate here: the owning process is already gone.
3. Delete the pod with `--force --grace-period=0` only after the mount entry is
   gone, then delete the cleanup pod.
4. If the unmount does not take effect, reboot the node.

Never unmount or delete mounts belonging to other pods; match the exact pod UID.

## Explicit non-claims

- The disposable PostgreSQL on `local-path` is a single instance and not HA;
  node-loss durability is not claimed.
- The privileged mount sidecar is a trusted component, not an agent sandbox.
  The unprivileged worker container is not claimed to be isolated from the
  workspace's contents.
- Certificate rotation requires a service restart (restart-only rotation).
- The shared `minio` namespace on the cluster is unrelated to the dev MinIO
  this proof installs.
- Published digests depend on Zot's version-specific retention behavior, checked
  by the readback procedure in [forgejo-delivery.md](forgejo-delivery.md) (G12).
- Not covered: spec 07 "lost completion response" (storage-side receipts are
  covered in-process), production execution-group termination verification,
  node loss, mmap and file locks, multi-replica service, Spaces, performance.
