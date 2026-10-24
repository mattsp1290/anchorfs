#!/usr/bin/env bash
# prepare-namespace.sh: owner-run on infra-control. Creates namespace anchorfs
# (idempotent), applies deploy/k8s/anchorfs-pki.yaml, waits for the three
# Certificates, and prints the key NAMES (never values) of the issued Secrets
# for gate G3. Requires KUBECONFIG (e.g. /etc/rancher/k3s/k3s.yaml).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

echo "client/server versions:"; k version | sed 's/^/  /'

if ! k get namespace "$NS" >/dev/null 2>&1; then
  k create namespace "$NS"
fi
# Worker pods run a privileged mount sidecar, so the namespace must allow it.
k label namespace "$NS" --overwrite \
  pod-security.kubernetes.io/enforce=privileged \
  app.kubernetes.io/managed-by=anchorfs-cluster-proof

k apply --namespace "$NS" -f "$root/deploy/k8s/anchorfs-pki.yaml"

for cert in anchorfs-ca anchorfs-service-tls anchorfs-minio-tls; do
  k wait --namespace "$NS" --for=condition=Ready "certificate/$cert" --timeout=120s
done

echo "G3: key names present in issued Secrets (values not printed):"
for sec in anchorfs-ca anchorfs-tls anchorfs-minio-tls; do
  printf '  %s:' "$sec"
  k get secret "$sec" --namespace "$NS" -o go-template='{{range $k, $_ := .data}} {{$k}}{{end}}{{"\n"}}'
done
