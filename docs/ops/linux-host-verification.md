# Disposable Linux mount verification

Run from a clean committed AnchorFS checkout. Replace `<host>` with the
owner-authorized SSH target; do not commit host identities or credentials.
Only disposable PostgreSQL/MinIO containers and mountpoints on local disk
are used. Existing infrastructure data is outside this workflow.

Before copying, check the host:

```sh
ssh <host> 'uname -r; id -u; ls -l /dev/fuse; fusermount3 --version; docker info --format "{{.ServerVersion}}"; rustup show; git --version'
```

The uid must be nonzero. The test harness fails if FUSE or fusermount3 is
missing. Verify `stat -f -c %T` for the effective temp directory, checkout and
Cargo target directory returns neither `nfs` nor `nfs4`. On the authorized
host `/tmp` and `~/anchorfs-verify` are on local disk.

Sync tracked files only, preserving the host's target cache:

```sh
git status --porcelain
git rev-parse HEAD
git ls-files -z | rsync -0a --files-from=- . <host>:anchorfs-verify/
```

Do not include untracked research, local settings or secrets. The archive is a
source snapshot, not a Git checkout. Record the source commit and local status
before the run; run from the host directory:

```sh
ssh <host> 'cd ~/anchorfs-verify && cargo test -p anchorfs-mount --locked -- --test-threads=1 > mount-tests.log 2>&1; task_result=$?; tail -100 mount-tests.log; exit $task_result'
ssh <host> 'cd ~/anchorfs-verify && cargo test --workspace --locked -- --test-threads=4 > workspace-tests.log 2>&1; task_result=$?; tail -100 workspace-tests.log; exit $task_result'
```

Every kernel acceptance test is serialized by one global mutex even under
`--test-threads=4`. The service has a dedicated runtime, the mount owns its
runtime, and blocking syscalls run on ordinary threads. Each mount has its own
watchdog and temporary directory. Normal teardown first tries non-lazy unmount,
then uses lazy unmount for failed-test cleanup, and kills only its own process.
The testcontainers reaper owns disposable container cleanup.

Record the kernel, source commit, command result, test summary and observed
unsupported-operation errors in the Stage 2b provenance record. The serial host
run and hosted CI are distinct evidence; passing either does not prove the
other or imply cross-node support.
