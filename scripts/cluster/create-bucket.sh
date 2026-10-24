#!/usr/bin/env bash
# create-bucket.sh: owner-run, after the dev MinIO StatefulSet is Ready. Runs a
# one-off pod from the pinned MinIO image whose `mc` client creates the bucket
# over verified TLS, then a second one-off pod that only lists it. Requires
# kubectl (KUBECTL, default `k3s kubectl`; install.sh exports its own value so
# both use the same client) and MINIO_IMAGE_DIGEST (sha256:<64 hex>); optional
# BUCKET and MINIO_IMAGE_REPO. Credentials reach the pod only through
# secretKeyRef env vars; the container builds the MC_HOST_dev URL itself, so the
# password is neither in kubectl nor in mc argv and nothing is echoed. The mc
# client ships in the pinned image on its PATH (verified 2026-10-22 in a local
# container, gate G2); with the upstream fallback image use a pinned minio/mc
# image instead. The CA Secret is mounted read-only at /etc/anchorfs-ca and
# copied into $MC_CONFIG_DIR/certs/CAs/ (a directory mc trusts) so that the
# writable config directory is created by the container user, not by a mount.
# `kubectl run --overrides` is applied as a strategic merge patch (the override
# container carries the generated name, so it merges with, rather than
# replaces, the generated container and its sh -c args). Any earlier one-off
# pods are deleted first.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
: "${MINIO_IMAGE_DIGEST:?set MINIO_IMAGE_DIGEST=sha256:<64 hex> of bitnamilegacy/minio}"
BUCKET="${BUCKET:-anchorfs}"
MINIO_IMAGE_REPO="${MINIO_IMAGE_REPO:-bitnamilegacy/minio}"

[[ "$MINIO_IMAGE_DIGEST" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo "MINIO_IMAGE_DIGEST must be sha256:<64 hex>" >&2; exit 2; }

k rollout status statefulset/anchorfs-dev-minio --namespace "$NS" --timeout=300s

pod=anchorfs-create-bucket
k delete pod "$pod" "${pod}-verify" --namespace "$NS" --ignore-not-found >/dev/null

read -r -d '' overrides <<JSON || true
{
  "spec": {
    "automountServiceAccountToken": false,
    "securityContext": {"runAsNonRoot": true, "runAsUser": 1001, "fsGroup": 1001,
                        "seccompProfile": {"type": "RuntimeDefault"}},
    "containers": [{
      "name": "$pod",
      "image": "${MINIO_IMAGE_REPO}@${MINIO_IMAGE_DIGEST}",
      "stdin": true,
      "env": [
        {"name": "MC_CONFIG_DIR", "value": "/tmp/mc"},
        {"name": "BUCKET", "value": "$BUCKET"},
        {"name": "MINIO_ROOT_USER", "valueFrom": {"secretKeyRef": {"name": "anchorfs-dev-minio", "key": "MINIO_ROOT_USER"}}},
        {"name": "MINIO_ROOT_PASSWORD", "valueFrom": {"secretKeyRef": {"name": "anchorfs-dev-minio", "key": "MINIO_ROOT_PASSWORD"}}}
      ],
      "securityContext": {"allowPrivilegeEscalation": false, "capabilities": {"drop": ["ALL"]}},
      "volumeMounts": [{"name": "ca", "mountPath": "/etc/anchorfs-ca", "readOnly": true}]
    }],
    "volumes": [{"name": "ca", "secret": {"secretName": "anchorfs-ca", "defaultMode": 288,
                 "items": [{"key": "tls.crt", "path": "ca.crt"}]}}]
  }
}
JSON

# Single-quoted on purpose: $VARS expand inside the container, never on the host.
prelude='mkdir -p "$MC_CONFIG_DIR/certs/CAs" && cp /etc/anchorfs-ca/ca.crt "$MC_CONFIG_DIR/certs/CAs/" && export MC_HOST_dev="https://${MINIO_ROOT_USER}:${MINIO_ROOT_PASSWORD}@anchorfs-minio:9000"'
create_script="$prelude"' && mc mb --ignore-existing "dev/$BUCKET" && mc ls "dev/$BUCKET"'
verify_script="$prelude"' && mc ls "dev/$BUCKET"'

k run "$pod" --namespace "$NS" --rm -i --restart=Never --pod-running-timeout=5m \
  --image="${MINIO_IMAGE_REPO}@${MINIO_IMAGE_DIGEST}" \
  --override-type=strategic --overrides="$overrides" -- sh -c "$create_script"

# Second, independent listing proves the bucket exists (no mb here).
k run "${pod}-verify" --namespace "$NS" --rm -i --restart=Never --pod-running-timeout=5m \
  --image="${MINIO_IMAGE_REPO}@${MINIO_IMAGE_DIGEST}" \
  --override-type=strategic --overrides="${overrides//\"$pod\"/\"${pod}-verify\"}" \
  -- sh -c "$verify_script" >/dev/null \
  || { echo "bucket $BUCKET could not be verified" >&2; exit 1; }
echo "bucket $BUCKET present"
