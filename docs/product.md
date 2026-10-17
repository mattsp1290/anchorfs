# Product direction

AnchorFS provides durable state independent of compute: multiple agent pods can share a persistent workspace while individual tool workers are replaced. A harness can access committed files without retaining a session-long execution worker.

## Confirmed requirements

- Rust implementation; prospective consumers include Crabber and eino-agent, not yet proven integrations.
- Local Kubernetes with MinIO and DOKS with DigitalOcean Spaces first; EC2, AWS S3 and Lambda access later.
- Multiple pods editing one workspace; no requirement that backing objects remain ordinary user files.
- Independent implementation with explicit cleanroom review gates.

## Baseline design and boundaries

The approved baseline chooses PostgreSQL metadata, immutable private chunks and synchronous object persistence. Those are independent design choices documented in the specs, not verified product capabilities or inferred competitor internals. Single operations serialize where necessary; multi-call workflows require explicit coordination. Session databases remain separate.

Initial exclusions include production remote execution, asynchronous durable write-back caching, snapshots, arbitrary bucket synchronization and a full S3 frontend. The storage backend is S3-compatible; that does not make AnchorFS an S3 API server. Do not expand a milestone to chase feature parity.

## Acceptance journey

A host identifies and authorizes a workspace. Worker A mounts it and commits file changes. The harness reads the committed contents through the API. Worker A exits. Worker B on another node attaches to the same workspace and reads those files without copy-back. Stale worker capabilities cannot publish new changes or create replacement sessions. Persistence claims are qualified by actual metadata/backend durability.

## Evaluation lenses

| Lens | Question |
|---|---|
| Durable publication | Do acknowledged mutations survive the promised failures without ambiguous replay? |
| Shared editing | Are cross-client visibility, namespace operations and conflict semantics correct? |
| Worker lifecycle and isolation | Can workers be replaced with access revoked and no false sandbox guarantee? |
| Provider and deployment portability | Is the same contract demonstrated on the supported backend/platform matrix? |
| Consumer boundary | Can a Rust/Go host adopt storage without replacing its session authority? |
| Independent provenance | Are requirements, code origins and compatibility claims reviewable within the approved input boundary? |

Current state and selected milestones do not live here. Rebuild them from code, tests and current plans on each selection. The staged delivery spec is direction, not proof that any stage is complete or a fully worked implementation plan.
