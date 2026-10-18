# anchorfs
durable state independent of compute

AnchorFS is an independent Rust storage project for persistent workspaces shared by agent harnesses and replaceable tool workers. The service implements authenticated workspace/session control, transactional namespace metadata and empty-file handles, plus a locally verified S3 backend adapter. Durable file bytes and Linux mounts remain later work. See [Stage 1b evidence](docs/provenance/stage1b-namespace-review.md).

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
containers; unavailable Docker fails the tests. No existing infrastructure data is used.
