# anchorfs
durable state independent of compute

AnchorFS is an independent Rust storage project for persistent workspaces shared by agent harnesses and replaceable tool workers. The service implements authenticated workspace/session control, transactional namespace metadata, file-byte operations and read pins against PostgreSQL and a locally verified S3 backend adapter. The route suite verifies 38 implemented operations with no stubs. Stage 2a adds exclusive workflow leases, host-controlled session binding and lease status; see [Stage 2a evidence](docs/provenance/stage2a-workflow-leases-review.md). Stage 1c local acceptance, independent review and hosted Linux/macOS gates passed. Stage 2b adds the strict Linux FUSE mount and a partial native Rust client; local kernel acceptance, review checkpoints and all five hosted jobs passed; see [Stage 2b evidence](docs/provenance/stage2b-linux-mount-review.md). See [Linux host verification](docs/ops/linux-host-verification.md). See [Stage 1c evidence](docs/provenance/stage1c-byte-path-review.md).

Start with [the implementation documents](docs/README.md) and [cleanroom policy](docs/cleanroom.md).

Repository-local workflows:

- `$anchorfs-next-milestone` selects one bounded outcome from approved requirements and current code/test evidence.
- `$anchorfs-implementation-plan` creates a concrete plan with two independent initial reviews and a fresh adversarial review. Every review explicitly checks cleanroom provenance as well as technical correctness.
- Invoke both to select and then plan. If your client does not discover local skills automatically, open their [milestone](.agents/skills/anchorfs-next-milestone/SKILL.md) and [planning](.agents/skills/anchorfs-implementation-plan/SKILL.md) entrypoints directly.

The approved initial direction is shared Linux workspaces backed by MinIO or DigitalOcean Spaces, with PostgreSQL metadata. AWS and other extensions are staged. This is not a claim of competitor feature parity or private-protocol compatibility. The [import manifest](docs/provenance/import-manifest.json) and [setup evidence](docs/provenance/setup-review.md) preserve the handoff identity and review limits.

## Building and testing

Use Rust 1.95.0 (rust-toolchain.toml). Run `cargo build --workspace --locked`,
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo deny check`, and `cargo test --workspace --locked` from the repository root.
Install the gate with `cargo install cargo-deny --locked --version 0.20.2`.
Integration tests require a running Docker daemon and create disposable PostgreSQL
and pinned MinIO containers; unavailable Docker fails the tests. No existing infrastructure data is used.

Mount an ordinary pre-minted session on Linux (the credential file contains the
bearer credential, not a CLI argument value):

```sh
anchorfs-mount mount /path/to/empty-mountpoint \
  --endpoint https://anchorfs.example.invalid --workspace <workspace-uuid> \
  --credential-file /path/to/credential \
  --session-id <session-uuid> --session-epoch <epoch>
```

A host credential may omit the session pair to self-mint an ordinary session.
For a lease-bound mount, provide the bound session plus `--workflow-token-file`
and `--workflow-epoch`. Use `--allow-plaintext` only when explicitly selecting
an HTTP endpoint. The service terminates TLS 1.3 when `ANCHORFS_TLS_CERT_PATH` and
`ANCHORFS_TLS_KEY_PATH` are set. See [container operations](docs/ops/container-images.md)
for packaging and listener configuration. The mount
is Linux-only; `--print-profile` and portable tests also work on macOS.
