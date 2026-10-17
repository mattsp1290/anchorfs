---
name: anchorfs-next-milestone
description: Select one bounded AnchorFS milestone from approved cleanroom specifications and current repository evidence, then optionally hand it to anchorfs-implementation-plan. Use when deciding what to build next, not for implementing a selected task or researching competitors.
---

# AnchorFS Next Milestone

Reconstruct the implementation frontier each run. This is an adaptation of select-next-milestone: approved requirements replace its mandatory live-comparator research and comparator metadata bootstrap. Do not run the generic skill or its validator to restore that lane.

## Modes

- Standalone: inspect read-only, present candidates, obtain selection and return one ready/blocked brief. Create no plan, issue, metadata or implementation artifact.
- Composed: complete selection and material decisions, then hand exactly one brief to the local [planning skill](../anchorfs-implementation-plan/SKILL.md). That skill owns context questions, plan files and required technical/cleanroom reviews.

## Approved inputs

Resolve the Git root from invocation directory; stop outside a worktree. Read applicable AGENTS.md and repository-relative `docs/product.md`, `docs/cleanroom.md`, `docs/README.md`, `docs/provenance/import-manifest.json` and relevant specs. Use approved local amendments when present; hashes identify the baseline, not all future requirements. If direction/approval is missing or conflicting, show the precise gap and ask the owner before a ready recommendation. Do not invent product vision.

Inspect relevant AnchorFS code paths, tests, configuration, CI, existing plans and tracker state. Consult explicitly authorized consumer public contracts read-only when a candidate needs them; avoid their research archives, private logs and competitor-derived internals. No sibling writes or live infrastructure mutations. Use official standards/platform and chosen licensed dependency documentation for technical uncertainties. Do not retrieve competitor pages, repositories, examples or private interfaces.

A missing competitor detail becomes a behavioral question for the requirements author outside this implementation context. Mark only candidates that actually depend on it evidence-blocked; unrelated approved work can continue. Do not require fresh competitor coverage to select from the approved baseline or claim this selection establishes parity.

## Evidence and frontier

Before candidate generation record HEAD, relevant dirty-file status/content hashes (including untracked inputs), approved spec/decision identities, existing plan identities, related contract revisions used and standard/dependency source versions/access dates. Do not read secret-bearing files to compute a fingerprint.

Label evidence `Repository fact`, `Approved requirement`, `User decision`, `Local ecosystem fact`, `External standard/platform fact`, `Inference`, or `Proposal`.

Classify each capability as `implemented`, `partial`, `planned`, `absent`, `blocked`, or `unknown`. `implemented` requires a real executable path plus a meaningful passing test or observed execution. Unrun tests, documentation, plan status, mocks and names prove only their own surfaces. Do not mark the whole storage product implemented because its spec or this setup exists.

Render one row per product evaluation lens:

| Lens/question | Current state | Repository proof | Approved requirement | Owner | Smallest gap | Acceptance-journey relevance |
|---|---|---|---|---|---|---|

Use existing plans as demand/status evidence, not automatic proof that a capability is implemented. Exclude fully planned duplicate outcomes from fresh candidates; offer to resume their exact plan separately. A broad staged delivery outline is not a fully planned milestone.

## Candidates and selection

Show two to four bounded candidates when evidence supports them, recommendation first. Explain when fewer are defensible. For every candidate give:

- Name and type (`user journey` or `enabling foundation`), beneficiary and one observable outcome.
- Applicable approved requirement/lenses and before/after acceptance flow, labeling existing versus proposed seams.
- Repository proof, proposed insertion point, owner and verified or unresolved dependencies.
- Readiness (`ready`, `foundation first`, `decision needed`, `upstream dependency`, `discovery`, or `planned`).
- Largest material decision with its evidence class, bounded scope/non-goals, safe verification and rank rationale.
- Provenance boundary: allowed inputs, any missing approval and explicit limit on compatibility claims.

Rank by nearest repeatable acceptance outcome or its only blocking foundation, alignment with declared vision, verified ownership, bounded credential-free proof, uncertainty reduction and scope size. Avoid speculative platform rewrites or feature-count parity.

Stop after showing fresh options and ask one concise selection question. Do not select for the user unless they delegate after seeing these options. Preserve a non-recommended choice and its trade-off. This selection workflow is not permission to build.

## Revalidate and resolve

After the pause, recheck relevant repository content/status and approved evidence identities. Material changes invalidate the displayed set and pending selection: rebuild affected candidates, redisplay and ask for fresh selection even if the recommendation remains the same. Recheck again immediately before handoff.

Ask at most three short questions at a time, only for material behavior, interfaces, ownership, persistence, recovery, security, lifecycle, platforms or scope. Honor explicit answers already applicable to this task. Unknown required dependencies or source approvals must identify the owner, affected journey and precise unblock evidence; no private imports or mocks masquerading as completion.

## Handoff

Return one brief with ready/blocked status, selected option and actual user decision, product outcome, relevant spec sections, evidence identities/classes, current/target flow, scope/exclusions, owners, dependency gates, decisions, exact verification and remaining questions. A ready brief needs resolved material decisions, permitted inputs and usable required contracts. Otherwise report a blocked brief with owner and exact action for each blocker.

In composed mode give that brief to `anchorfs-implementation-plan`; do not create an interim roadmap or alternative plan format. The resulting plan must complete two independent initial reviews and a fresh adversarial review, each explicitly checking cleanroom principles. Never imply those reviews happened merely because the brief requests them.
