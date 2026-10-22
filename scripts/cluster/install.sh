#!/usr/bin/env bash
# install.sh <publication.json>: owner-run on infra-control. Installs release
# "anchorfs" in namespace anchorfs from the chart blob fetched from the Zot
# Distribution API BY DIGEST and verified with SHA-256 (no oci:// reference and
# no --version: Helm 3.13 cannot resolve oci://...@sha256: and no tag exists).
# Then creates the MinIO bucket, runs `helm test`, and bootstraps the
# administrator once into Secret anchorfs-bootstrap-admin. Prints names and
# digests only; the bootstrap credential goes straight into a Secret.
# Receipt shape (birbhomes publisher): .artifacts.{service,mount,chart}.{repository,digest}.
set -euo pipefail

: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
KUBECTL="${KUBECTL:-k3s kubectl}"
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
chart_path="${chart_repo#*/}"   # strip host:port, keep e.g. charts/anchorfs

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
tgz="$work/anchorfs-0.1.0.tgz"
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
k create secret generic anchorfs-bootstrap-admin --namespace "$NS" --from-file=credential="$cred" >/dev/null
echo "created Secret anchorfs-bootstrap-admin (credential not printed)"
