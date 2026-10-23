#!/usr/bin/env bash
# create-secrets.sh [--rotate]: owner-run. Generates random credentials and
# creates the Secrets the chart references: anchorfs-dev-postgres,
# anchorfs-dev-minio, anchorfs-service-env. Needs openssl and kubectl. Values
# are never printed and never appear in kubectl argv: they are written with the
# printf builtin to KEY=value files in a umask-077 temporary directory (removed
# on exit) and passed with --from-env-file. If all three Secrets exist nothing
# changes; if only some exist (an interrupted earlier run) the script fails
# naming the missing ones; --rotate deletes the existing ones and replaces all
# three together, because the service env embeds the database and MinIO
# credentials (a rotated database password only works against a fresh database
# volume).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

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
  if [ "${#existing[@]}" -lt "${#names[@]}" ]; then
    missing=()
    for n in "${names[@]}"; do
      case " ${existing[*]} " in *" $n "*) ;; *) missing+=("$n") ;; esac
    done
    echo "partial Secret set: present (${existing[*]}), missing (${missing[*]}); run with --rotate to replace all three" >&2
    exit 1
  fi
  echo "Secrets already exist (${existing[*]}); pass --rotate to replace them. Nothing changed."
  exit 0
fi
if [ "${#existing[@]}" -gt 0 ]; then
  for n in "${existing[@]}"; do k delete secret "$n" --namespace "$NS"; done
fi

umask 077
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

rand() { openssl rand -hex 32; }
pg_password="$(rand)"
minio_user="anchorfs$(openssl rand -hex 6)"
minio_password="$(rand)"
cursor_key="$(rand)"   # 64 hex characters, as the service requires

printf 'POSTGRES_PASSWORD=%s\n' "$pg_password" >"$tmp/postgres.env"
printf 'MINIO_ROOT_USER=%s\nMINIO_ROOT_PASSWORD=%s\n' "$minio_user" "$minio_password" >"$tmp/minio.env"
printf 'ANCHORFS_DATABASE_URL=postgres://anchorfs:%s@anchorfs-dev-postgres:5432/anchorfs\nANCHORFS_CURSOR_SIGNING_KEY=%s\nANCHORFS_S3_ACCESS_KEY_ID=%s\nANCHORFS_S3_SECRET_ACCESS_KEY=%s\n' \
  "$pg_password" "$cursor_key" "$minio_user" "$minio_password" >"$tmp/service.env"

k create secret generic anchorfs-dev-postgres --namespace "$NS" --from-env-file="$tmp/postgres.env" >/dev/null
k create secret generic anchorfs-dev-minio --namespace "$NS" --from-env-file="$tmp/minio.env" >/dev/null
k create secret generic anchorfs-service-env --namespace "$NS" --from-env-file="$tmp/service.env" >/dev/null

echo "created secrets: ${names[*]} (values not printed)"
