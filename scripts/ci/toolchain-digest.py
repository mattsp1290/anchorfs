"""Print the toolchain digest recorded for the accepted CI job image.

The v1 build receipt consumed by the shared publisher requires a syntactically
valid `toolchain_digest`. While the app policy's builder pins are null the value
is informational; it is still derived from a committed source so the receipt
is reproducible. The digest below is the SHA-256 of the birbhomes CI image's
`toolchain.lock.json` as recorded in `deploy/contracts/builder.json` at the
helper pin (see docs/ops/forgejo-delivery.md). If the job image exposes its
lock file at a documented path, that file's digest wins and must equal the
recorded value.

Run with `python3 -I scripts/ci/toolchain-digest.py`; standard library only.
"""
import hashlib
import pathlib
import re
import sys

RECORDED = "sha256:d95a89dd6120aef69a35ad53bfb002c372753a9df8d42fbafadb18db4d4da3eb"
CANDIDATE_PATHS = (
    "/opt/birbhomes/toolchain.lock.json",
    "/opt/birbhomes/ci-image/toolchain.lock.json",
)


def main() -> int:
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", RECORDED):
        print("invalid recorded digest", file=sys.stderr)
        return 2
    for candidate in CANDIDATE_PATHS:
        path = pathlib.Path(candidate)
        if path.is_file():
            actual = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
            if actual != RECORDED:
                print(f"toolchain lock at {candidate} has digest {actual}, expected {RECORDED}", file=sys.stderr)
                return 2
            break
    print(RECORDED)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
