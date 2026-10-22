#!/usr/bin/env bash
# create-bucket.sh: owner-run, after the dev MinIO StatefulSet is Ready. Runs a
# one-off pod from the pinned MinIO image whose `mc` client creates the bucket
# over verified TLS, then lists it a second time. Credentials reach the pod only
# through secretKeyRef; nothing is echoed. The mc client ships in the pinned
# image at /opt/bitnami/minio-client/bin/mc, on its PATH (verified 2026-10-22
# in a local container, gate G2); with the upstream fallback image use a
# pinned minio/mc image instead. mc trusts CAs placed under
# $MC_CONFIG_DIR/certs/CAs/, which is where the CA Secret is mounted.
# Requires MINIO_IMAGE_DIGEST (sha256:<64 hex>).
set -euo pipefail

: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
: "${MINIO_IMAGE_DIGEST:?set MINIO_IMAGE_DIGEST=sha256:<64 hex> of bitnamilegacy/minio}"
KUBECTL="${KUBECTL:-k3s kubectl}"
NS=anchorfs
BUCKET="${BUCKET:-anchorfs}"
MINIO_IMAGE_REPO="${MINIO_IMAGE_REPO:-bitnamilegacy/minio}"
k() { $KUBECTL "$@"; }

[[ "$MINIO_IMAGE_DIGEST" =~ ^sha256:[a-f0-9]{64}$ ]] || { echo "MINIO_IMAGE_DIGEST must be sha256:<64 hex>" >&2; exit 2; }

k rollout status statefulset/anchorfs-dev-minio --namespace "$NS" --timeout=300s

pod=anchorfs-create-bucket
k delete pod "$pod" --namespace "$NS" --ignore-not-found >/dev/null

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
      "volumeMounts": [{"name": "ca", "mountPath": "/tmp/mc/certs/CAs", "readOnly": true}]
    }],
    "volumes": [{"name": "ca", "secret": {"secretName": "anchorfs-ca", "defaultMode": 288,
                 "items": [{"key": "tls.crt", "path": "ca.crt"}]}}]
  }
}
JSON

script='mc alias set dev https://anchorfs-minio:9000 "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD" >/dev/null && mc mb --ignore-existing "dev/$BUCKET" && mc ls "dev/$BUCKET"'
k run "$pod" --namespace "$NS" --rm -i --restart=Never \
  --image="${MINIO_IMAGE_REPO}@${MINIO_IMAGE_DIGEST}" --overrides="$overrides" -- sh -c "$script"

# Second, independent listing proves the bucket exists.
k run "${pod}-verify" --namespace "$NS" --rm -i --restart=Never \
  --image="${MINIO_IMAGE_REPO}@${MINIO_IMAGE_DIGEST}" \
  --overrides="${overrides//\"$pod\"/\"${pod}-verify\"}" -- sh -c "$script" >/dev/null \
  || { echo "bucket $BUCKET could not be verified" >&2; exit 1; }
echo "bucket $BUCKET present"
