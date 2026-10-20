# AnchorFS implementation documents

The seven files in `spec/` are the reviewed requirements handoff, copied byte-for-byte on 2026-10-17. [The import manifest](provenance/import-manifest.json) pins their source identities. These describe intended behavior; no storage implementation exists at bootstrap.

Read [product direction](product.md) and [cleanroom policy](cleanroom.md), then:

1. [Requirements](spec/01-requirements.md)
2. [Storage and concurrency](spec/02-storage.md)
3. [Interfaces and filesystem semantics](spec/03-interfaces.md)
4. [Deployment and consumers](spec/04-deployment.md)
5. [Delivery and acceptance](spec/05-delivery.md)
6. [Original provenance boundary](spec/06-provenance.md)
7. [Harness and worker lifecycle](spec/07-tool-lifecycle.md)

The imported files' original `spec/` wording means `docs/spec/` in this repository. Their references to external research and prior reviews describe historical evidence, not permission to retrieve it. This repository's [cleanroom policy](cleanroom.md) governs operational input handling; if a technical requirement conflicts with later decisions, record and resolve the conflict rather than silently rewriting the baseline.

Only the implementation-safe specifications were imported. Competitor research, pasted post prose, infrastructure inventory and historical research-facing reviews remain outside this repository. A new behavioral question goes to the requirements author for an approved independent specification.

Use the [local milestone selector](../.agents/skills/anchorfs-next-milestone/SKILL.md) and [local planning skill](../.agents/skills/anchorfs-implementation-plan/SKILL.md). [Setup review evidence](provenance/setup-review.md) records workflow adaptation and validation; it is not a claim that runtime behavior has been tested.

## Implementation artifacts

- [Native API contract](api/openapi-v1.yaml), generated from service route annotations.
- [Persistent state ownership](design/persistent-state.md) and [frozen contracts](design/contracts.md).
- [Dependency provenance](provenance/dependencies.md), [Stage 0 review evidence](provenance/stage0-review.md), and [Stage 1 backend evidence](provenance/stage1-backend-review.md).

Backend object operations are implemented and verified against a pinned local MinIO image, and readiness checks database settings and backend list capability. The authenticated control plane implements thirteen routes for workspace creation/inspection, credentials, sessions and operation receipts, verified by disposable PostgreSQL HTTP and transaction tests. Stage 1b adds ten namespace routes and three metadata-only handle routes (26 implemented / 12 stubs), server permissions, logical quota accounting and authenticated list cursors, verified against disposable PostgreSQL and a sequential model. Stage 1c connects write, append, truncate, fsync, verified reads, streamed conditional replacement and read pins (34 implemented / 4 workflow-lease stubs), with byte conformance against disposable PostgreSQL and MinIO. Stage 1c has local byte conformance, model and process-restart evidence; independent review and hosted CI remain pending; Linux mounts remain later work. See [the incremental Stage 1c record](provenance/stage1c-byte-path-review.md). See [Stage 1a evidence](provenance/stage1a-control-plane-review.md) and [Stage 1b evidence](provenance/stage1b-namespace-review.md).
