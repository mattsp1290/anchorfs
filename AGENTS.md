# Working in AnchorFS

AnchorFS is an independent Rust implementation of durable workspaces shared across replaceable compute. Read [the documentation index](docs/README.md), [product direction](docs/product.md), and [cleanroom policy](docs/cleanroom.md) before planning or implementation.

## Planning skills

- Use `.agents/skills/anchorfs-next-milestone/SKILL.md` to select the next bounded milestone from approved requirements and actual repository evidence.
- Use `.agents/skills/anchorfs-implementation-plan/SKILL.md` to create a reviewed implementation plan.
- Compose the two for selection followed by planning. Do not use the generic milestone skill's live competitor research workflow in this repository.

## Cleanroom input and review rules

The approved implementation inputs are `docs/spec/`, AnchorFS code/tests/design decisions, explicit user requirements, and permitted standards/platform/dependency sources under `docs/cleanroom.md`. The external research archive is not an implementer input. Do not inspect competitor implementation repositories, SDK implementation, binaries, traces, private protocols, or source-facing research reports to fill gaps.

Every plan and code review must explicitly assess provenance, independent design, prohibited-source exposure, dependency licenses and unsupported compatibility claims using the cleanroom policy's review checklist. Reviewers must receive fresh context and an explicit approved-input packet. If that cannot be provided, report the missing review stage rather than silently inheriting research history. Fresh context is procedural separation, not proof of legal clearance or erased historical exposure.

Treat unknown source origin or prohibited exposure as a blocker for the affected artifact. Preserve an origin/impact record without reproducing sensitive or prohibited material, stop affected work, and route behavioral clarification through the requirements author. Unaffected work may continue. Never silently copy competitor details or claim deleting a file erases exposure.

## Evidence and scope

Specifications and plans prove intent, not implemented behavior. Require executable paths and meaningful passing tests or observed execution before claiming a capability implemented. No storage code or build gate exists at bootstrap; add real verification with implementation. Never run destructive tests against existing infrastructure data.

Keep new code/design/dependency provenance in `docs/provenance/`. Imported specification hashes record the original handoff; review later amendments separately. Do not modify imported baselines silently to make tests pass. Changes to product scope belong in a recorded requirements decision.
