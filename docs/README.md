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

Backend object operations are implemented and verified against a pinned local MinIO image, and readiness checks database settings and backend list capability. The authenticated control plane implements thirteen routes for workspace creation/inspection, credentials, sessions and operation receipts, verified by disposable PostgreSQL HTTP and transaction tests. Stage 1b adds ten namespace routes and three metadata-only handle routes (26 implemented / 12 stubs), server permissions, logical quota accounting and authenticated list cursors, verified against disposable PostgreSQL and a sequential model. Stage 1c connects write, append, truncate, fsync, verified reads, streamed conditional replacement and read pins (34 byte-era implemented routes (four lease routes added in Stage 2a)), with byte conformance against disposable PostgreSQL and MinIO. Stage 1c has local byte conformance, model and process-restart evidence; both independent review stages, local fixer gates and hosted Linux/macOS CI passed; Linux mounts were deferred to Stage 2b. See [the incremental Stage 1c record](provenance/stage1c-byte-path-review.md). See [Stage 1a evidence](provenance/stage1a-control-plane-review.md) and [Stage 1b evidence](provenance/stage1b-namespace-review.md).

Stage 2a implements the four workflow lease routes, lease-bound session mint/rebind and workspace lease inspection (38 implemented / 0 stubs). See [Stage 2a evidence](provenance/stage2a-workflow-leases-review.md) and the [executable acceptance map](provenance/stage2a-workflow-leases-acceptance.md). Linux mounts were deferred to Stage 2b.

Stage 2b adds `anchorfs-client`, explicit inode timestamps and the strict Linux
FUSE mount. Local kernel acceptance, both review checkpoints and all five hosted
CI jobs passed; see
[Stage 2b evidence](provenance/stage2b-linux-mount-review.md),
[the executable acceptance map](provenance/stage2b-linux-mount-acceptance.md)
and [the Linux host workflow](ops/linux-host-verification.md). Single-node
process visibility is the scope; cross-node deployment remains later work.

Stage 3a adds an in-process TLS 1.3 listener, native client and Linux mount
HTTPS acceptance, and native amd64/arm64 container build jobs. See
[container operations](ops/container-images.md),
[Stage 3a review evidence](provenance/stage3a-tls-and-images-review.md), and
[Stage 3a acceptance](provenance/stage3a-tls-and-images-acceptance.md) for actual
executed status and limitations. Stage 3a itself published no image; Stage 3b
later published the two runtime bases (below). No cluster or cross-node
deployment is claimed.

Stage 3b prepares the delivery path and cluster proof: a Forgejo workflow that
publishes digest-addressed images and an OCI Helm chart to the internal Zot
registry, and an owner-run two-node proof. See
[Forgejo delivery](ops/forgejo-delivery.md), [cluster proof](ops/cluster-proof.md),
[container operations](ops/container-images.md),
[Stage 3b review evidence](provenance/stage3b-delivery-and-cluster-review.md) and
[Stage 3b acceptance](provenance/stage3b-delivery-and-cluster-acceptance.md).
Non-claims: no Flux promotion, no HA, no Spaces, no sandbox. No service, mount
or chart artifact is published and nothing is deployed; the only AnchorFS
content in the internal Zot registry is the two owner-published runtime bases
(`anchorfs/service-base`, `anchorfs/mount-base`, see `docker/bases.json`). The
Forgejo and cluster rows are pending.
