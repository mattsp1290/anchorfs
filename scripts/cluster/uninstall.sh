#!/usr/bin/env bash
# uninstall.sh [--delete-data] [--delete-pki]: owner-run. Removes proof worker
# pods and the helm release in namespace anchorfs only. --delete-data also
# deletes the dev PVCs and the Secrets created by create-secrets.sh/install.sh;
# --delete-pki also deletes the cert-manager objects, their Secrets and the
# namespace labels. Without flags it states what it left behind. Never touches
# other namespaces.
set -euo pipefail

: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
KUBECTL="${KUBECTL:-k3s kubectl}"
HELM="${HELM:-helm}"
NS=anchorfs
k() { $KUBECTL "$@"; }

delete_data=0; delete_pki=0
for arg in "$@"; do
  case "$arg" in
    --delete-data) delete_data=1 ;;
    --delete-pki) delete_pki=1 ;;
    *) echo "usage: $0 [--delete-data] [--delete-pki]" >&2; exit 2 ;;
  esac
done

k get namespace "$NS" >/dev/null 2>&1 || { echo "namespace $NS does not exist; nothing to do"; exit 0; }

k delete pod --namespace "$NS" -l anchorfs.proof/role=worker --ignore-not-found
if $HELM status anchorfs --namespace "$NS" >/dev/null 2>&1; then
  $HELM uninstall anchorfs --namespace "$NS"
else
  echo "helm release anchorfs not found"
fi

if [ "$delete_data" -eq 1 ]; then
  k delete pvc --namespace "$NS" -l app.kubernetes.io/name=anchorfs --ignore-not-found
  k delete secret --namespace "$NS" --ignore-not-found \
    anchorfs-dev-postgres anchorfs-dev-minio anchorfs-service-env anchorfs-bootstrap-admin
else
  echo "left behind: dev PVCs and Secrets anchorfs-dev-postgres anchorfs-dev-minio anchorfs-service-env anchorfs-bootstrap-admin (use --delete-data)"
fi

if [ "$delete_pki" -eq 1 ]; then
  k delete certificate --namespace "$NS" --ignore-not-found anchorfs-service-tls anchorfs-minio-tls anchorfs-ca
  k delete issuer --namespace "$NS" --ignore-not-found anchorfs-ca anchorfs-selfsigned
  k delete secret --namespace "$NS" --ignore-not-found anchorfs-tls anchorfs-minio-tls anchorfs-ca
  k label namespace "$NS" pod-security.kubernetes.io/enforce- app.kubernetes.io/managed-by-
else
  echo "left behind: cert-manager Issuers/Certificates, their Secrets and namespace labels (use --delete-pki)"
fi
echo "namespace $NS itself is kept"
