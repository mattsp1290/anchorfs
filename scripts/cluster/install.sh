#!/usr/bin/env bash
# install.sh <publication.json>: owner-run on infra-control. Installs release
# "anchorfs" in namespace anchorfs from the chart blob fetched from the Zot
# Distribution API BY DIGEST and verified with SHA-256 (no oci:// reference and
# no --version: Helm 3.13 cannot resolve oci://...@sha256: and no tag exists).
# Then creates the MinIO bucket, runs `helm test`, and bootstraps the
# administrator once into Secret anchorfs-bootstrap-admin. Prints names and
# digests only; the bootstrap credential goes straight into a Secret.
# Receipt shape (birbhomes publisher; docs/ops/forgejo-delivery.md):
# .artifacts.{service,mount,chart}.{repository,digest}, .artifact_set_digest,
# .source_sha, .run_id. Each repository must equal exactly
# <registry host:port>/anchorfs/service, .../anchorfs/mount and
# .../charts/anchorfs (host:port taken from REGISTRY_URL), so a receipt cannot
# inject helm values or redirect pulls. The receipt's source_sha, run_id and
# artifact_set_digest formats are checked and printed so the operator can
# compare them with the recorded main run; set EXPECTED_SOURCE_SHA (40 hex) to
# have the script compare source_sha itself. Host prerequisites: kubectl
# (KUBECTL, exported for create-bucket.sh), helm 3.13+, jq, curl, and
# sha256sum or shasum. The chart archive is a temporary file named by its layer
# digest. If storing the bootstrap Secret fails after the credential was
# minted, the credential file is saved with mode 600 under
# ${XDG_RUNTIME_DIR:-$HOME} and only its path is printed.
set -euo pipefail

: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
KUBECTL="${KUBECTL:-k3s kubectl}"
export KUBECTL
HELM="${HELM:-helm}"
NS=anchorfs
REGISTRY_URL="${REGISTRY_URL:-http://10.0.0.106:30001}"
# Digests of the pinned dev dependency images (same pins as the integration tests).
POSTGRES_IMAGE_DIGEST="${POSTGRES_IMAGE_DIGEST:-sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3}"
MINIO_IMAGE_DIGEST="${MINIO_IMAGE_DIGEST:-sha256:6dabb4a2088c9a79908de3bc05f4586c23ad2182c8908e7e3acbf61c1467fb20}"
export MINIO_IMAGE_DIGEST
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
k() { $KUBECTL "$@"; }

[ "$#" -eq 1 ] || { echo "usage: $0 <publication.json>" >&2; exit 2; }
receipt="$1"
[ -r "$receipt" ] || { echo "cannot read $receipt" >&2; exit 2; }

sha256_of() { # file -> hex digest
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
digest_re='^sha256:[a-f0-9]{64}$'
field() { jq -er ".artifacts.$1.$2" "$receipt"; }

service_repo="$(field service repository)"; service_digest="$(field service digest)"
mount_repo="$(field mount repository)";     mount_digest="$(field mount digest)"
chart_repo="$(field chart repository)";     chart_digest="$(field chart digest)"
for d in "$service_digest" "$mount_digest" "$chart_digest" "$POSTGRES_IMAGE_DIGEST" "$MINIO_IMAGE_DIGEST"; do
  [[ "$d" =~ $digest_re ]] || { echo "malformed digest in inputs: $d" >&2; exit 2; }
done

registry_hostport="${REGISTRY_URL#*://}"; registry_hostport="${registry_hostport%%/*}"
expect_repo() { [ "$1" = "$2" ] || { echo "unexpected repository for $3: $1 (expected $2)" >&2; exit 2; }; }
expect_repo "$service_repo" "$registry_hostport/anchorfs/service" service
expect_repo "$mount_repo"   "$registry_hostport/anchorfs/mount"   mount
expect_repo "$chart_repo"   "$registry_hostport/charts/anchorfs"  chart
source_sha="$(jq -er '.source_sha' "$receipt")"
run_id="$(jq -er '.run_id' "$receipt")"
set_digest="$(jq -er '.artifact_set_digest' "$receipt")"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "malformed source_sha in receipt" >&2; exit 2; }
[[ "$run_id" =~ ^[0-9]+$ ]] || { echo "malformed run_id in receipt" >&2; exit 2; }
[[ "$set_digest" =~ $digest_re ]] || { echo "malformed artifact_set_digest in receipt" >&2; exit 2; }
if [ -n "${EXPECTED_SOURCE_SHA:-}" ]; then
  [ "$source_sha" = "$EXPECTED_SOURCE_SHA" ] || { echo "receipt source_sha $source_sha does not equal EXPECTED_SOURCE_SHA" >&2; exit 2; }
fi
chart_path="${chart_repo#*/}"   # strip host:port, keep e.g. charts/anchorfs
echo "receipt  source_sha ${source_sha} run_id ${run_id} artifact_set ${set_digest}"

echo "versions:"; k version | sed 's/^/  /'; $HELM version --short | sed 's/^/  helm /'
echo "service  ${service_repo}@${service_digest}"
echo "mount    ${mount_repo}@${mount_digest}"
echo "chart    ${chart_path}@${chart_digest} via ${REGISTRY_URL}"

work="$(mktemp -d)"; chmod 700 "$work"
trap 'rm -rf "$work"' EXIT

# 1. Chart manifest by digest; body hash must equal the digest.
curl -fsS -H 'Accept: application/vnd.oci.image.manifest.v1+json' \
  -o "$work/manifest.json" "$REGISTRY_URL/v2/$chart_path/manifests/$chart_digest"
[ "sha256:$(sha256_of "$work/manifest.json")" = "$chart_digest" ] \
  || { echo "chart manifest digest mismatch" >&2; exit 1; }

# 2. Single Helm chart layer; verify media type, then blob hash and size.
[ "$(jq '.layers | length' "$work/manifest.json")" = 1 ] || { echo "expected exactly one layer" >&2; exit 1; }
[ "$(jq -r '.layers[0].mediaType' "$work/manifest.json")" = application/vnd.cncf.helm.chart.content.v1.tar+gzip ] \
  || { echo "unexpected chart layer media type" >&2; exit 1; }
layer_digest="$(jq -r '.layers[0].digest' "$work/manifest.json")"
layer_size="$(jq -r '.layers[0].size' "$work/manifest.json")"
[[ "$layer_digest" =~ $digest_re ]] || { echo "malformed layer digest" >&2; exit 1; }
tgz="$work/chart-${layer_digest#sha256:}.tgz"
curl -fsS -o "$tgz" "$REGISTRY_URL/v2/$chart_path/blobs/$layer_digest"
[ "sha256:$(sha256_of "$tgz")" = "$layer_digest" ] || { echo "chart layer digest mismatch" >&2; exit 1; }
[ "$(wc -c <"$tgz" | tr -d ' ')" = "$layer_size" ] || { echo "chart layer size mismatch" >&2; exit 1; }
echo "installed chart identity: manifest ${chart_digest}, tgz ${layer_digest}"

# 3. Install from the verified file.
$HELM upgrade --install anchorfs "$tgz" --namespace "$NS" \
  -f "$here/values-apartment.yaml" \
  --set "image.service.repository=${service_repo}" --set "image.service.digest=${service_digest}" \
  --set "image.mount.repository=${mount_repo}" --set "image.mount.digest=${mount_digest}" \
  --set devDependencies.enabled=true \
  --set "devDependencies.postgres.image.digest=${POSTGRES_IMAGE_DIGEST}" \
  --set "devDependencies.minio.image.digest=${MINIO_IMAGE_DIGEST}" \
  --history-max 5
# No `--wait` here on purpose: the service's readiness probe requires the S3
# bucket, which only exists after create-bucket.sh, so waiting for the
# Deployment before the bucket would never complete.

# 4. Dependencies up, bucket created, then the service becomes ready.
k rollout status statefulset/anchorfs-dev-postgres --namespace "$NS" --timeout=300s
"$here/create-bucket.sh"   # waits for the MinIO StatefulSet itself
k rollout status deployment/anchorfs --namespace "$NS" --timeout=10m
$HELM test anchorfs --namespace "$NS" --timeout 5m --logs

# 5. Bootstrap the administrator once; the credential never reaches the terminal.
if k get secret anchorfs-bootstrap-admin --namespace "$NS" >/dev/null 2>&1; then
  echo "Secret anchorfs-bootstrap-admin exists; administrator already bootstrapped"
  exit 0
fi
umask 077
cred="$work/credential.json"
rc=0
k exec --namespace "$NS" deploy/anchorfs -- anchorfs-service bootstrap-admin --uid 10001 --gid 10001 >"$cred" || rc=$?
if [ "$rc" -eq 8 ]; then
  echo "bootstrap-admin exit 8 (administrator already exists) but Secret anchorfs-bootstrap-admin is missing: recover the credential manually" >&2
  exit 1
elif [ "$rc" -ne 0 ]; then
  echo "bootstrap-admin failed with exit $rc" >&2
  exit "$rc"
fi
stored=0
for attempt in 1 2 3; do
  if k create secret generic anchorfs-bootstrap-admin --namespace "$NS" --from-file=credential="$cred" >/dev/null; then stored=1; break; fi
  sleep 3
done
if [ "$stored" -ne 1 ]; then
  keep="${XDG_RUNTIME_DIR:-$HOME}/anchorfs-bootstrap-admin.json"
  install -m 600 "$cred" "$keep"
  echo "could not create Secret anchorfs-bootstrap-admin; the credential was saved (mode 600) at $keep. Create the Secret from it, then delete the file." >&2
  exit 1
fi
echo "created Secret anchorfs-bootstrap-admin (credential not printed)"
