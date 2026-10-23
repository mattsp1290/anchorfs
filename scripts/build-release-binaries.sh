#!/usr/bin/env bash
# Cross-builds the release binaries for both supported Linux targets with
# cargo-zigbuild and places them under dist/linux-{amd64,arm64}/.
#
# The ".2.36" suffix is the glibc version of Debian bookworm, the runtime base
# of docker/Dockerfile. Change both together.
#
# Needs: cargo, cargo-zigbuild, zig, python3 (scripts/oci_export.py check-elf; stdlib only).
# No daemon, no registry, no network beyond cargo's own crate fetches.
set -euo pipefail

cd "$(dirname "$0")/.."

GLIBC=2.36
TARGETS=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu)
PACKAGES=(anchorfs-service anchorfs-mount)

die() { echo "build-release-binaries: $*" >&2; exit 1; }
# shellcheck source=ci/sha256.sh
source scripts/ci/sha256.sh

for tool in cargo cargo-zigbuild zig python3; do
  command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done
echo "cargo-zigbuild $(cargo-zigbuild --version | awk '{print $2}'); zig $(zig version)" >&2

args=()
for target in "${TARGETS[@]}"; do
  args+=(--target "${target}.${GLIBC}")
done
for package in "${PACKAGES[@]}"; do
  args+=(-p "$package")
done
cargo zigbuild --locked --release "${args[@]}"

place() {
  local arch=$1 triple=$2
  mkdir -p "dist/linux-$arch"
  for package in "${PACKAGES[@]}"; do
    local src="target/$triple/release/$package"
    [ -f "$src" ] || die "missing build output: $src"
    python3 -I scripts/oci_export.py check-elf "$src" "$arch" || die "$src is not a 64-bit little-endian ELF for $arch"
    install -m 0755 "$src" "dist/linux-$arch/$package"
    local sum
    sum=$(sha256_of "dist/linux-$arch/$package") || die "hashing failed for dist/linux-$arch/$package"
    echo "dist/linux-$arch/$package $sum" >&2
  done
}

place amd64 x86_64-unknown-linux-gnu
place arm64 aarch64-unknown-linux-gnu
echo "built both targets at glibc $GLIBC" >&2
