#!/usr/bin/env bash
# Owner-run: builds the service-base and mount-base Dockerfile stages for
# linux/amd64 + linux/arm64, pushes each as an OCI image index to the apartment
# LAN registry with crane, reads the index back, and records the index and
# per-platform manifest digests in docker/bases.json. Never run from CI.
# The owner reviews the bases.json diff and commits it (a provenance record).
#
# Usage: [CRANE_IMAGE=<image>] scripts/publish-bases.sh [--replace]
# Needs: git, docker buildx (multi-platform), crane, python3 (stdlib only), tar.
# Orchestration only: stage hashing, readback verification and every bases.json
# read/write are subcommands of scripts/oci_export.py (the single owner).
# Refuses: dirty tree, non-LAN registry host, missing tools, already-recorded
# bases without --replace, a CRANE_IMAGE that is not pinned (see below). Prints
# digests only. Also records `stage_sha256` per base: the SHA-256 of the exact
# Dockerfile bytes of the base stage (its `FROM ... AS <target>` line up to, not
# including, the next `FROM` line; `oci_export.py stage-sha256`). A provenance test recomputes it, so editing a
# base stage without re-running this script fails the test.
#
# CRANE_IMAGE: when set, every crane call runs inside `docker run --rm
# --entrypoint crane "$CRANE_IMAGE"`; the OCI tarball buildx produced is
# streamed to the push container over stdin (no bind mounts, so this works with
# Docker Desktop, Colima and remote daemons alike). The push and readback then
# use the Docker VM's network instead of the host's. Use it on a macOS shell
# that macOS does not grant Local Network access (the symptom is crane failing
# with "no route to host" while curl succeeds); the pinned birbhomes CI image
# carries crane. Unset, the host crane is used. When set, CRANE_IMAGE must be
# either a reference containing `@sha256:<64 hex>` or a Docker image ID (12 to
# 64 hex characters, optionally prefixed `sha256:`); a mutable tag is refused.
set -euo pipefail

cd "$(dirname "$0")/.."

die() { echo "publish-bases: $*" >&2; exit 1; }
export_py() { python3 -I scripts/oci_export.py "$@"; }

replace=0
case "${1:-}" in
  "") ;;
  --replace) replace=1 ;;
  *) die "usage: $0 [--replace]" ;;
esac
[ "$#" -le 1 ] || die "usage: $0 [--replace]"

for tool in git docker python3 tar; do
  command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done
[ -n "${CRANE_IMAGE:-}" ] || command -v crane >/dev/null 2>&1 || die "missing tool: crane (or set CRANE_IMAGE)"
if [ -n "${CRANE_IMAGE:-}" ]; then
  [[ $CRANE_IMAGE =~ @sha256:[0-9a-f]{64}($|[^0-9a-f]) || $CRANE_IMAGE =~ ^(sha256:)?[0-9a-f]{12,64}$ ]] \
    || die "CRANE_IMAGE must be pinned: a reference with @sha256:<64 hex> or a Docker image ID"
fi
docker buildx version >/dev/null 2>&1 || die "docker buildx is required"

[ -z "$(git status --porcelain --untracked-files=all)" ] || die "working tree is not clean; commit first"
source_sha=$(git rev-parse HEAD)

registry=$(export_py bases registry docker/bases.json) || die "cannot read docker/bases.json"
host=${registry%%:*}
# Only RFC 1918 IPv4 literals are accepted as the registry host.
if ! [[ $host =~ ^(10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|192\.168\.[0-9]{1,3}\.[0-9]{1,3}|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3})$ ]]; then
  die "registry host '$host' is not a LAN address"
fi
[[ $registry =~ ^[0-9.]+:[0-9]{2,5}$ ]] || die "registry '$registry' must be host:port"

bases_list=$(export_py bases list docker/bases.json) || die "cannot read docker/bases.json"
if [ "$replace" -eq 0 ] && grep -qvE ' none$' <<<"$bases_list"; then
  die "docker/bases.json already records a base digest; pass --replace to publish again"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# crane on the host, or inside CRANE_IMAGE (read-only commands: no mounts).
run_crane() {
  if [ -n "${CRANE_IMAGE:-}" ]; then
    docker run --rm --entrypoint crane "$CRANE_IMAGE" "$@"
  else
    crane "$@"
  fi
}
# push_layout <base> <ref>: pushes the buildx OCI layout; in container mode the
# tarball is streamed over stdin and unpacked inside the container.
push_layout() {
  if [ -n "${CRANE_IMAGE:-}" ]; then
    docker run --rm -i --entrypoint sh "$CRANE_IMAGE" -c \
      'set -e; d=$(mktemp -d); tar -xf - -C "$d"; exec crane push --insecure "$d" "$1"' \
      sh "$2" < "$work/$1.tar"
  else
    crane push --insecure "$work/$1" "$2"
  fi
}
# Reachability first, before spending time on builds.
run_crane catalog --insecure "$registry" >/dev/null \
  || die "cannot reach registry $registry with crane${CRANE_IMAGE:+ (inside $CRANE_IMAGE)}; on macOS a shell without Local Network permission fails with 'no route to host' while curl works: grant the permission or set CRANE_IMAGE to the pinned CI image"

recorded=()
for base in service mount; do
  read -r _ repository target _ < <(grep "^$base " <<<"$bases_list")
  ref="$registry/$repository"
  stage_sha256=$(export_py stage-sha256 docker/Dockerfile "$target") || die "cannot hash Dockerfile stage $target"
  echo "building $target for linux/amd64,linux/arm64" >&2
  docker buildx build -f docker/Dockerfile --platform linux/amd64,linux/arm64 --target "$target" \
    --provenance=false --sbom=false \
    --output "type=oci,dest=$work/$base.tar" .
  mkdir "$work/$base"
  tar -xf "$work/$base.tar" -C "$work/$base"

  # The layout's index.json has exactly one entry: the multi-platform index.
  expected=$(export_py layout-digest "$work/$base/index.json") || die "$base: layout index.json must hold exactly one entry"
  # Without --index, crane pushes that single entry (the index) directly; --index
  # would wrap it in a second index.
  push_layout "$base" "$ref" >/dev/null
  run_crane manifest --insecure "$ref@$expected" > "$work/$base.manifest.json"
  # Verify the index SHA-256 and platform set, then fetch every platform manifest
  # by digest and verify its SHA-256, so a partially pushed index cannot be recorded.
  platform_digests=$(export_py verify-readback index "$expected" "$work/$base.manifest.json") \
    || die "$base: readback of the pushed index failed verification"
  while read -r _ pdigest; do
    run_crane manifest --insecure "$ref@$pdigest" > "$work/$base.platform.json"
    export_py verify-readback platform "$pdigest" "$work/$base.platform.json" \
      || die "$base: platform manifest $pdigest failed readback verification"
  done <<<"$platform_digests"
  recorded+=("$base $expected $stage_sha256")
done

# Both bases are pushed and verified; only now is docker/bases.json rewritten.
for entry in "${recorded[@]}"; do
  read -r base expected stage_sha256 <<<"$entry"
  export_py bases record docker/bases.json "$base" "$expected" "$work/$base.manifest.json" "$stage_sha256" "$source_sha"
done

echo "docker/bases.json updated; review the diff and commit it promptly." >&2
if [ "$replace" -eq 1 ]; then
  echo "note: the 'latest' tag moved to the new indexes; the previous digests stay served only while the registry retains untagged manifests." >&2
fi
