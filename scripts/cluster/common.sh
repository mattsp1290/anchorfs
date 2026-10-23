# Sourced by the other scripts in this directory (not run directly): the shared
# prelude. Requires KUBECONFIG; KUBECTL defaults to `k3s kubectl` and is
# exported so a script that calls another one (install.sh -> create-bucket.sh)
# hands it the same client. Provides NS, k() and sha256_of.
: "${KUBECONFIG:?set KUBECONFIG, e.g. /etc/rancher/k3s/k3s.yaml}"
KUBECTL="${KUBECTL:-k3s kubectl}"
export KUBECTL
NS=anchorfs
k() { $KUBECTL "$@"; }
# shellcheck source=../ci/sha256.sh
source "$(dirname "${BASH_SOURCE[0]}")/../ci/sha256.sh"
