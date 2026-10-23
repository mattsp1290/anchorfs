#!/usr/bin/env bash
# Owner-run: builds the service-base and mount-base Dockerfile stages for
# linux/amd64 + linux/arm64, pushes each as an OCI image index to the apartment
# LAN registry with crane, reads the index back, and records the index and
# per-platform manifest digests in docker/bases.json. Never run from CI.
# The owner reviews the bases.json diff and commits it (a provenance record).
#
# Usage: [CRANE_IMAGE=<image>] scripts/publish-bases.sh [--replace]
# Needs: git, docker buildx (multi-platform), crane, python3 (stdlib only), tar.
# Refuses: dirty tree, non-LAN registry host, missing tools, already-recorded
# bases without --replace. Prints digests only.
#
# CRANE_IMAGE: when set, every crane call runs inside `docker run --rm
# --entrypoint crane "$CRANE_IMAGE"`; the OCI tarball buildx produced is
# streamed to the push container over stdin (no bind mounts, so this works with
# Docker Desktop, Colima and remote daemons alike). The push and readback then
# use the Docker VM's network instead of the host's. Use it on a macOS shell
# that macOS does not grant Local Network access (the symptom is crane failing
# with "no route to host" while curl succeeds); the pinned birbhomes CI image
# carries crane. Unset, the host crane is used.
set -euo pipefail

cd "$(dirname "$0")/.."

die() { echo "publish-bases: $*" >&2; exit 1; }

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
docker buildx version >/dev/null 2>&1 || die "docker buildx is required"

[ -z "$(git status --porcelain --untracked-files=all)" ] || die "working tree is not clean; commit first"
source_sha=$(git rev-parse HEAD)

registry=$(python3 -I -c 'import json,sys; print(json.load(open("docker/bases.json"))["registry"])')
host=${registry%%:*}
# Only RFC 1918 IPv4 literals are accepted as the registry host.
if ! [[ $host =~ ^(10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|192\.168\.[0-9]{1,3}\.[0-9]{1,3}|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3})$ ]]; then
  die "registry host '$host' is not a LAN address"
fi
[[ $registry =~ ^[0-9.]+:[0-9]{2,5}$ ]] || die "registry '$registry' must be host:port"

if [ "$replace" -eq 0 ]; then
  python3 -I - <<'PY' || die "docker/bases.json already records a base digest; pass --replace to publish again"
import json, sys
bases = json.load(open("docker/bases.json"))["bases"]
sys.exit(1 if any(b["index_digest"] is not None for b in bases.values()) else 0)
PY
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
results="$work/results.json"
echo '{}' > "$results"

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

for base in service mount; do
  read -r repository target old < <(python3 -I - "$base" <<'PY'
import json, sys
b = json.load(open("docker/bases.json"))["bases"][sys.argv[1]]
print(b["repository"], b["dockerfile_target"], b["index_digest"] or "none")
PY
)
  ref="$registry/$repository"
  echo "building $target for linux/amd64,linux/arm64" >&2
  docker buildx build -f docker/Dockerfile --platform linux/amd64,linux/arm64 --target "$target" \
    --provenance=false --sbom=false \
    --output "type=oci,dest=$work/$base.tar" .
  mkdir "$work/$base"
  tar -xf "$work/$base.tar" -C "$work/$base"

  # The layout's index.json has exactly one entry: the multi-platform index.
  expected=$(python3 -I - "$work/$base/index.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))["manifests"]
if len(m) != 1:
    sys.exit("layout index.json must hold exactly one entry")
print(m[0]["digest"])
PY
)
  # Without --index, crane pushes that single entry (the index) directly; --index
  # would wrap it in a second index.
  push_layout "$base" "$ref" >/dev/null
  run_crane manifest --insecure "$ref@$expected" > "$work/$base.manifest.json"
  python3 -I - "$base" "$expected" "$work/$base.manifest.json" "$results" "$old" <<'PY'
import hashlib, json, sys
base, expected, path, results, old = sys.argv[1:]
raw = open(path, "rb").read()
if "sha256:" + hashlib.sha256(raw).hexdigest() != expected:
    sys.exit("readback digest mismatch")
index = json.loads(raw)
platforms = {}
for m in index["manifests"]:
    p = m["platform"]
    platforms[p["os"] + "/" + p["architecture"]] = m["digest"]
if sorted(platforms) != ["linux/amd64", "linux/arm64"] or len(index["manifests"]) != 2:
    sys.exit("readback index must list exactly linux/amd64 and linux/arm64")
data = json.load(open(results))
data[base] = {"index_digest": expected, "platforms": platforms}
json.dump(data, open(results, "w"))
print(f"{base}: old {old}")
print(f"{base}: new {expected}")
for k in sorted(platforms):
    print(f"{base}: {k} {platforms[k]}")
PY
done

python3 -I - "$results" "$source_sha" <<'PY'
import datetime, json, sys
results = json.load(open(sys.argv[1]))
doc = json.load(open("docker/bases.json"))
now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
for name, r in results.items():
    b = doc["bases"][name]
    b["index_digest"] = r["index_digest"]
    b["platforms"] = r["platforms"]
    b["published_at"] = now
    b["source_sha"] = sys.argv[2]
with open("docker/bases.json", "w") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
PY
echo "docker/bases.json updated; review the diff and commit it promptly." >&2
if [ "$replace" -eq 1 ]; then
  echo "note: the 'latest' tag moved to the new indexes; the previous digests stay served only while the registry retains untagged manifests." >&2
fi
