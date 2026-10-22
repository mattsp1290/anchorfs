#!/usr/bin/env bash
# Owner-run: builds the service-base and mount-base Dockerfile stages for
# linux/amd64 + linux/arm64, pushes each as an OCI image index to the apartment
# LAN registry with crane, reads the index back, and records the index and
# per-platform manifest digests in docker/bases.json. Never run from CI.
# The owner reviews the bases.json diff and commits it (a provenance record).
#
# Usage: scripts/publish-bases.sh [--replace]
# Needs: git, docker buildx (multi-platform), crane, python3 (stdlib only), tar.
# Refuses: dirty tree, non-LAN registry host, missing tools, already-recorded
# bases without --replace. Prints digests only.
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

for tool in git docker crane python3 tar; do
  command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done
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
  crane push --insecure "$work/$base" "$ref" >/dev/null
  crane manifest --insecure "$ref@$expected" > "$work/$base.manifest.json"
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
