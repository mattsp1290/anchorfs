#!/usr/bin/env bash
# create-secrets.sh [--rotate]: owner-run. Generates random credentials and
# creates the Secrets the chart references: anchorfs-dev-postgres,
# anchorfs-dev-minio, anchorfs-service-env. Values are never printed. Existing
# Secrets are left alone unless --rotate (which replaces all three together,
# because the service env embeds the database and MinIO credentials; a rotated
# database password only works against a fresh database volume).
set -euo pipefail

: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
KUBECTL="${KUBECTL:-k3s kubectl}"
NS=anchorfs
k() { $KUBECTL "$@"; }

rotate=0
case "${1:-}" in
  "") ;;
  --rotate) rotate=1 ;;
  *) echo "usage: $0 [--rotate]" >&2; exit 2 ;;
esac

names=(anchorfs-dev-postgres anchorfs-dev-minio anchorfs-service-env)
existing=()
for n in "${names[@]}"; do
  if k get secret "$n" --namespace "$NS" >/dev/null 2>&1; then existing+=("$n"); fi
done
if [ "${#existing[@]}" -gt 0 ] && [ "$rotate" -eq 0 ]; then
  echo "Secrets already exist (${existing[*]}); pass --rotate to replace them. Nothing changed."
  exit 0
fi
if [ "${#existing[@]}" -gt 0 ]; then
  for n in "${existing[@]}"; do k delete secret "$n" --namespace "$NS"; done
fi

rand() { openssl rand -hex 32; }
pg_password="$(rand)"
minio_user="anchorfs$(openssl rand -hex 6)"
minio_password="$(rand)"
cursor_key="$(rand)"   # 64 hex characters, as the service requires

k create secret generic anchorfs-dev-postgres --namespace "$NS" \
  --from-literal=POSTGRES_PASSWORD="$pg_password" >/dev/null
k create secret generic anchorfs-dev-minio --namespace "$NS" \
  --from-literal=MINIO_ROOT_USER="$minio_user" \
  --from-literal=MINIO_ROOT_PASSWORD="$minio_password" >/dev/null
k create secret generic anchorfs-service-env --namespace "$NS" \
  --from-literal=ANCHORFS_DATABASE_URL="postgres://anchorfs:${pg_password}@anchorfs-dev-postgres:5432/anchorfs" \
  --from-literal=ANCHORFS_CURSOR_SIGNING_KEY="$cursor_key" \
  --from-literal=ANCHORFS_S3_ACCESS_KEY_ID="$minio_user" \
  --from-literal=ANCHORFS_S3_SECRET_ACCESS_KEY="$minio_password" >/dev/null

echo "created secrets: ${names[*]} (values not printed)"
