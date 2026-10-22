#!/usr/bin/env bash
# Cross-builds the release binaries for both supported Linux targets with
# cargo-zigbuild and places them under dist/linux-{amd64,arm64}/.
#
# The ".2.36" suffix is the glibc version of Debian bookworm, the runtime base
# of docker/Dockerfile. Change both together.
#
# Needs: cargo, cargo-zigbuild, zig, python3 (ELF machine check; stdlib only).
# No daemon, no registry, no network beyond cargo's own crate fetches.
set -euo pipefail

cd "$(dirname "$0")/.."

GLIBC=2.36
TARGETS=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu)
PACKAGES=(anchorfs-service anchorfs-mount)

die() { echo "build-release-binaries: $*" >&2; exit 1; }

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

# arch name -> cargo target triple, ELF e_machine (EM_X86_64 = 0x3e, EM_AARCH64 = 0xb7)
check_elf() {
  python3 -I - "$1" "$2" <<'PY'
import struct, sys
path, expected = sys.argv[1], int(sys.argv[2], 16)
with open(path, "rb") as f:
    header = f.read(20)
if header[:4] != b"\x7fELF" or header[4] != 2:
    sys.exit(f"{path}: not a 64-bit ELF file")
machine = struct.unpack_from("<H", header, 18)[0]
if machine != expected:
    sys.exit(f"{path}: e_machine 0x{machine:x}, expected 0x{expected:x}")
PY
}

place() {
  local arch=$1 triple=$2 machine=$3
  mkdir -p "dist/linux-$arch"
  for package in "${PACKAGES[@]}"; do
    local src="target/$triple/release/$package"
    [ -f "$src" ] || die "missing build output: $src"
    check_elf "$src" "$machine"
    install -m 0755 "$src" "dist/linux-$arch/$package"
    echo "dist/linux-$arch/$package $(shasum -a 256 "dist/linux-$arch/$package" | cut -d' ' -f1)" >&2
  done
}

place amd64 x86_64-unknown-linux-gnu 3e
place arm64 aarch64-unknown-linux-gnu b7
echo "built both targets at glibc $GLIBC" >&2
