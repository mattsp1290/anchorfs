#!/usr/bin/env bash
# check-chart.sh: render charts/anchorfs with synthetic digests and assert the
# Stage 3b chart invariants (kinds, security contexts, worker template, failure
# paths). Runs under `helm` and, when present, `helm-compat` (the Helm 3.13
# binary). CHART_STRICT=1 makes a missing helm-compat a failure. Needs only
# bash, awk, grep and helm; it never contacts a cluster. Rendering passes
# --kube-version explicitly on `template` (default: the apartment cluster's
# k3s version) because Chart.yaml's kubeVersion constraint is checked against
# Helm's built-in default otherwise, and Helm 3.13 defaults to v1.28. Helm
# 3.13's `lint` has no such flag and does not evaluate the constraint.
set -euo pipefail
KUBE_VERSION="${CHART_KUBE_VERSION:-1.35.9}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="$root/charts/anchorfs"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

d1="sha256:$(printf '1%.0s' $(seq 64))"
d2="sha256:$(printf '2%.0s' $(seq 64))"
d3="sha256:$(printf '3%.0s' $(seq 64))"
d4="sha256:$(printf '4%.0s' $(seq 64))"
base_set=(--set image.service.repository=registry.invalid/anchorfs/service --set "image.service.digest=$d1"
          --set image.mount.repository=registry.invalid/anchorfs/mount --set "image.mount.digest=$d2")
dev_set=(--set devDependencies.enabled=true --set "devDependencies.postgres.image.digest=$d3"
         --set "devDependencies.minio.image.digest=$d4")

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
ok() { echo "ok: $*"; }
# expect_count <file> <fixed-or-regex> <n> <label>
expect_count() {
  local n
  n="$(grep -Ec -e "$2" "$1" || true)"
  if [ "$n" = "$3" ]; then ok "$4"; else fail "$4 (expected $3, found $n of '$2')"; fi
}

run_with() {
  local bin="$1" out="$tmp/$1"
  mkdir -p "$out"
  echo "== $bin $("$bin" version --short 2>/dev/null || true)"

  if "$bin" lint "$chart" "${base_set[@]}" >"$out/lint.txt" 2>&1; then ok "lint passes"; else fail "lint failed"; cat "$out/lint.txt" >&2; fi

  local kv=(--kube-version "$KUBE_VERSION")
  "$bin" template anchorfs "$chart" -n anchorfs "${kv[@]}" "${base_set[@]}" >"$out/default.yaml"
  "$bin" template anchorfs "$chart" -n anchorfs "${kv[@]}" "${base_set[@]}" "${dev_set[@]}" >"$out/dev.yaml"
  "$bin" template anchorfs "$chart" -n anchorfs "${kv[@]}" "${base_set[@]}" --set 'networkPolicy.extraIngressCidrs={10.0.0.106/32}' >"$out/cidr.yaml"

  local f="$out/default.yaml"
  # Split into one file per document, named by kind and metadata.name.
  mkdir -p "$out/docs"
  awk -v dir="$out/docs" 'BEGIN{n=0; f=dir "/doc0"} /^---$/{n++; f=dir "/doc" n; next} {print > f}' "$f"

  for img in $(grep -E '^[[:space:]]+image:' "$f" | awk '{print $2}' | tr -d '"'); do
    case "$img" in *@sha256:*) ;; *) fail "image not by digest: $img";; esac
  done
  ok "every image reference checked for @sha256:"

  expect_count "$f" '^kind: Deployment$' 1 "one Deployment"
  expect_count "$f" '^kind: Service$' 1 "one Service"
  expect_count "$f" '^kind: ServiceAccount$' 1 "one ServiceAccount"
  expect_count "$f" '^kind: NetworkPolicy$' 1 "one NetworkPolicy"
  expect_count "$f" '^kind: ConfigMap$' 1 "one ConfigMap"
  expect_count "$f" '^  name: anchorfs-worker-template$' 1 "ConfigMap is anchorfs-worker-template"
  expect_count "$f" '^kind: Pod$' 1 "one Pod (test hook)"
  expect_count "$f" '^kind: (Secret|Issuer|ClusterIssuer|Certificate|Ingress|Role|RoleBinding|ClusterRole|ClusterRoleBinding|Job|StatefulSet|PersistentVolumeClaim)$' 0 "no forbidden kinds by default"

  # Service container security context and probes (Deployment document only).
  local dep
  dep="$(grep -l '^kind: Deployment$' "$out"/docs/doc* | head -1)"
  expect_count "$dep" 'runAsNonRoot: true' 2 "Deployment pod and container runAsNonRoot"
  expect_count "$dep" 'readOnlyRootFilesystem: true' 1 "Deployment readOnlyRootFilesystem"
  expect_count "$dep" 'allowPrivilegeEscalation: false' 1 "Deployment allowPrivilegeEscalation false"
  expect_count "$dep" 'drop: \["ALL"\]|^ *- ALL$' 1 "Deployment drops ALL capabilities"
  expect_count "$dep" 'path: /readyz' 2 "startup and readiness probes use /readyz"
  expect_count "$dep" 'path: /healthz' 1 "liveness probe uses /healthz"
  expect_count "$dep" 'scheme: HTTPS' 3 "all probes use HTTPS"
  expect_count "$dep" 'ANCHORFS_TLS_CERT_PATH' 1 "TLS cert path configured"
  expect_count "$dep" 'automountServiceAccountToken: false' 1 "Deployment does not mount an API token"

  expect_count "$f" 'helm.sh/hook: test' 1 "test hook annotation"
  local tp
  tp="$(grep -l 'name: anchorfs-test-probe' "$out"/docs/doc* | head -1)"
  expect_count "$tp" '^        - "https://anchorfs:8080"$' 1 "probe url is the origin only"
  expect_count "$tp" '^        - --require-(tls|durable)$' 2 "probe requires TLS and durability"

  # Worker template: extract the embedded pod manifest.
  local wt
  wt="$(grep -l 'name: anchorfs-worker-template' "$out"/docs/doc* | head -1)"
  expect_count "$wt" '^ +privileged: true$' 1 "worker template: privileged exactly once"
  expect_count "$wt" 'mountPropagation: Bidirectional' 1 "worker template: Bidirectional exactly once"
  expect_count "$wt" 'mountPropagation: HostToContainer' 1 "worker template: HostToContainer exactly once"
  expect_count "$wt" '^ +fsGroup: 10001$' 1 "worker template: fsGroup 10001 once"
  local secrets modes
  secrets="$(grep -c '^ *secret:$' "$wt" || true)"
  modes="$(grep -c 'defaultMode: 288$' "$wt" || true)"
  if [ "$secrets" = "$modes" ] && [ "$secrets" -ge 2 ]; then ok "worker template: every secret volume has defaultMode 288 ($secrets)"; else fail "worker template: secret volumes=$secrets defaultMode 288=$modes"; fi
  expect_count "$wt" '^ +emptyDir:$' 2 "worker template: two emptyDirs"
  expect_count "$wt" 'sizeLimit: 64Mi' 2 "worker template: sizeLimit on both emptyDirs"
  expect_count "$wt" 'medium: Memory' 1 "worker template: memory-backed workspace"
  expect_count "$wt" "registry.invalid/anchorfs/mount@$d2" 2 "worker template: mount digest bound in both containers"
  expect_count "$wt" 'automountServiceAccountToken: false' 1 "worker template: no API token"
  for ph in __NAME__ __NODE__ __WORKSPACE__ __SESSION_ID__ __SESSION_EPOCH__ __CREDENTIAL_SECRET__; do
    if grep -q "$ph" "$wt"; then ok "placeholder $ph present"; else fail "placeholder $ph missing"; fi
  done
  local block
  block="$(awk '/^ +- name: mount$/{p=1; next} /^ +- name: worker$/{p=0} p' "$wt")"
  if printf '%s\n' "$block" | grep -q 'allowPrivilegeEscalation'; then fail "mount container sets allowPrivilegeEscalation (breaks setuid fusermount3)"; else ok "mount container has no allowPrivilegeEscalation"; fi
  if [ -z "$block" ]; then fail "mount container block not found"; fi
  # The credential FILE path contains "bearer"; reject only literal values.
  if grep -Eqi 'password|secret_access_key|(bearer|token): [0-9a-f]{16}' "$wt"; then fail "worker template contains something credential-like"; else ok "no plaintext credential in worker template"; fi

  # Dev dependencies.
  expect_count "$out/dev.yaml" '^kind: StatefulSet$' 2 "dev: exactly two StatefulSets"
  expect_count "$out/dev.yaml" '^kind: Service$' 3 "dev: three Services (service plus two dependencies)"
  expect_count "$out/dev.yaml" '^kind: (Secret|Issuer|ClusterIssuer|Certificate|Ingress|Job)$' 0 "dev: no forbidden kinds"
  expect_count "$out/dev.yaml" "postgres@$d3" 1 "dev: postgres image by digest"
  expect_count "$out/dev.yaml" "bitnamilegacy/minio@$d4" 1 "dev: minio image by digest"

  expect_count "$out/cidr.yaml" 'cidr: "10.0.0.106/32"' 1 "extraIngressCidrs renders an ipBlock"

  # Failure paths.
  if "$bin" template anchorfs "$chart" "${kv[@]}" --set image.service.repository=x --set image.mount.repository=x --set "image.mount.digest=$d2" >"$out/e1.txt" 2>&1; then
    fail "empty service digest rendered"
  elif grep -q 'image.service.digest' "$out/e1.txt"; then ok "empty digest fails naming image.service.digest"; else fail "empty digest error does not name image.service.digest"; fi
  if "$bin" template anchorfs "$chart" "${kv[@]}" --set image.service.repository=x --set image.service.digest=latest --set image.mount.repository=x --set "image.mount.digest=$d2" >"$out/e2.txt" 2>&1; then
    fail "non-sha256 digest rendered"
  elif grep -q 'sha256' "$out/e2.txt"; then ok "non-sha256 digest rejected"; else fail "non-sha256 digest error unclear"; fi
}

command -v helm >/dev/null || { echo "helm not found" >&2; exit 2; }
run_with helm
if command -v helm-compat >/dev/null; then
  run_with helm-compat
elif [ "${CHART_STRICT:-0}" = "1" ]; then
  fail "helm-compat not found and CHART_STRICT=1"
else
  echo "note: helm-compat not found; Helm 3.13 compatibility not exercised"
fi

if [ "$failures" -ne 0 ]; then echo "$failures check(s) failed" >&2; exit 1; fi
echo "all chart checks passed"
