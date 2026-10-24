#!/usr/bin/env bash
# guard.sh <tool>...: first step of every delivery job, run right after checkout
# (this file and the workflow come from the same verified commit). Fails before any
# build when a required tool is missing, the host is not linux/aarch64, Python is
# older than 3.11, the pinned Rust toolchain is absent (when rustc is requested),
# the checked-out HEAD is not the 40-hex event SHA, or the ref is not
# refs/heads/main. Needs EXPECTED_REVISION and EVENT_REF in the environment
# (the workflow maps them from github.sha and github.ref).
set -euo pipefail

for tool in "$@"; do
  command -v "$tool" >/dev/null 2>&1 || { printf 'Missing CI prerequisite: %s\n' "$tool" >&2; exit 1; }
  case "$tool" in
    rustc) rustc --version | grep -E '^rustc 1\.95\.0 ' >/dev/null || { echo 'unexpected rustc version' >&2; exit 1; } ;;
    python3) python3 -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else "Python 3.11 or newer required")' ;;
  esac
done
test "$(uname -s)" = Linux && test "$(uname -m)" = aarch64 || { echo 'unsupported CI host' >&2; exit 1; }

: "${EXPECTED_REVISION:?}" "${EVENT_REF:?}"
case "$EXPECTED_REVISION" in ''|*[!0-9a-f]*) echo invalid_expected_revision >&2; exit 1 ;; esac
test "${#EXPECTED_REVISION}" -eq 40 || { echo invalid_expected_revision >&2; exit 1; }
test "$(git rev-parse --verify HEAD)" = "$EXPECTED_REVISION" || { echo checkout_revision_mismatch >&2; exit 1; }
test "$EVENT_REF" = refs/heads/main || { echo untrusted_ref >&2; exit 1; }
printf 'verified_revision=%s\n' "$EXPECTED_REVISION"
