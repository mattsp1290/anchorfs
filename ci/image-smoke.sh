set -euo pipefail
# 1 shipped contract equals the committed snapshot (trailing whitespace normalized; the Rust snapshot test uses trim_end)
tmp="${RUNNER_TEMP:-$(mktemp -d)}"
docker run --rm anchorfs-service:ci openapi --format yaml > "$tmp/openapi.yaml"
diff -u <(sed -e 's/[[:space:]]*$//' docs/api/openapi-v1.yaml) <(sed -e 's/[[:space:]]*$//' "$tmp/openapi.yaml")
# 2,3 non-root
test "$(docker run --rm --entrypoint id anchorfs-service:ci -u)" = 10001
test "$(docker run --rm --entrypoint id anchorfs-mount:ci -u)" = 10001
test "$(docker run --rm --entrypoint id anchorfs-service:ci -g)" = 10001
test "$(docker run --rm --entrypoint id anchorfs-mount:ci -g)" = 10001
# 4 bare serve fails closed on configuration (exit 2, names the missing variable)
rc=0; err=$(docker run --rm anchorfs-service:ci serve 2>&1 >/dev/null) || rc=$?
test "$rc" -eq 2; grep -q 'ANCHORFS_DATABASE_URL' <<<"$err"
# 5 a lone TLS variable is rejected before the database URL is required (exit 2, names the rule)
rc=0; err=$(docker run --rm -e ANCHORFS_TLS_CERT_PATH=/nonexistent anchorfs-service:ci serve 2>&1 >/dev/null) || rc=$?
test "$rc" -eq 2; grep -q 'set together' <<<"$err"
# 6 mount binary runs and prints the strict profile
out=$(docker run --rm anchorfs-mount:ci --print-profile); grep -q direct_io <<<"$out"
# 7 mount helper present
docker run --rm --entrypoint fusermount3 anchorfs-mount:ci --version
# 8 CA store present in the service image
docker run --rm --entrypoint sh anchorfs-service:ci -c 'test -r /etc/ssl/certs/ca-certificates.crt'
# 9 both shipped architectures and the host CPU must match the native job.
expected="${1:?expected image architecture (amd64 or arm64)}"
for image in anchorfs-service:ci anchorfs-mount:ci; do
  arch=$(docker image inspect "$image" --format '{{.Architecture}}')
  echo "$image architecture: $arch"
  test "$arch" = "$expected"
done
normalize_arch() {
  case "$1" in
    x86_64|amd64) echo amd64 ;;
    aarch64|arm64) echo arm64 ;;
    *) return 1 ;;
  esac
}
host=$(normalize_arch "$(uname -m)")
inside=$(docker run --rm --entrypoint uname anchorfs-service:ci -m)
test "$host" = "$expected"
test "$(normalize_arch "$inside")" = "$host"
