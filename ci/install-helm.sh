#!/usr/bin/env bash
# Installs the pinned Helm 3.13.0 release binary (the version the apartment
# control host and the birbhomes promoter fixture run) into a private directory
# and verifies the official SHA-256 before extracting. Used by the GitHub
# `packaging` job so chart checks run against the same Helm generation as the
# cluster. Checksums were read from https://get.helm.sh/helm-v3.13.0-linux-<arch>.tar.gz.sha256sum
# on 2026-10-22 and are recorded in docs/provenance/dependencies.md.
#
#   bash ci/install-helm.sh <install-dir>   # prints the helm path on stdout
set -euo pipefail

VERSION=v3.13.0
dest=${1:?install directory}
case "$(uname -m)" in
  x86_64|amd64) arch=amd64; expected=138676351483e61d12dfade70da6c03d471bbdcac84eaadeb5e1d06fa114a24f ;;
  aarch64|arm64) arch=arm64; expected=d12a0e73a7dbff7d89d13e0c6eb73f5095f72d70faea30531941d320678904d2 ;;
  *) echo "install-helm: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
[ "$(uname -s)" = Linux ] || { echo "install-helm: Linux only" >&2; exit 1; }

mkdir -p "$dest"
archive="$dest/helm-$VERSION-linux-$arch.tar.gz"
curl -fsSL --retry 3 -o "$archive" "https://get.helm.sh/helm-$VERSION-linux-$arch.tar.gz"
actual=$(sha256sum "$archive" | cut -d' ' -f1)
if [ "$actual" != "$expected" ]; then
  echo "install-helm: checksum mismatch for $archive" >&2
  rm -f "$archive"
  exit 1
fi
tar -xzf "$archive" -C "$dest" "linux-$arch/helm"
install -m 0755 "$dest/linux-$arch/helm" "$dest/helm"
rm -rf "$archive" "$dest/linux-$arch"
"$dest/helm" version --short | grep -q "^$VERSION" || { echo "install-helm: unexpected helm version" >&2; exit 1; }
echo "$dest/helm"
