---
name: anchorfs-implementation-plan
description: Create a repository-grounded AnchorFS implementation plan with two independent reviews, a fresh adversarial review, and explicit cleanroom provenance gates. Use for detailed planning or after anchorfs-next-milestone; not for immediate implementation.
---

# AnchorFS Implementation Plan

Planning is the deliverable. Do not implement, deploy, push, or change consumer repositories unless separately requested. This local adaptation uses approved requirements instead of live competitor research.

## Establish inputs and context

1. Resolve the Git root from the invocation directory and read applicable AGENTS.md. Stop outside a worktree. Read repository-relative `docs/README.md`, `docs/product.md`, `docs/cleanroom.md`, and relevant `docs/spec/` sections. Do not assume the invocation directory is the root.
2. Inspect relevant AnchorFS code, tests, configuration, current plans and worktree state. Follow interfaces enough to distinguish proposed behavior from implemented behavior. Use official standards/platform or reviewed dependency documentation when needed; never browse competitor sources or the external research archive.
3. If composed with `anchorfs-next-milestone`, consume exactly one selected brief and preserve the user's selection and accepted trade-off. A supplied fixed task does not require milestone selection. Do not convert a broad delivery-stage outline into an implementation-ready plan without concrete contracts and tests.
4. Confirm whether there are active users/external consumers and whether backward compatibility is required. Ask both questions and wait for explicit answers unless the current task already has applicable explicit answers. If either is yes, ask whether feature flags are `appropriate`, `not-appropriate`, or `decide-per-pr`. Both no means `not-applicable`. Never infer these from repository age, old setup answers, or absent code.
5. Record the exact `application_context` schema and digest from [plan-format.md](references/plan-format.md). Materially changed context requires fresh answers. Missing required answers or provenance/ownership decisions leave status blocked with an owner and precise unblock action.

## Write the plan

Normalize the name to one lowercase kebab-case segment (`^[a-z0-9]+(?:-[a-z0-9]+)*$`). Use `<git-root>/.agents/plans/<name>/`; resolve symlinks and verify it is a direct child of the resolved in-repository plans root. Reject escaping ancestors, absolute names and traversal. Preserve existing files unless revising this same plan is authorized; ask when their relationship is unclear.

Read [plan-format.md](references/plan-format.md). Write numbered Markdown files: overview, one or more cohesive work packages, and execution handoff. Include exact repository-relative change surfaces, evidence, invariants, lifecycle/error handling, dependencies, tests, rollout/rollback requirements and observable acceptance. Mark new/proposed files and symbols honestly. Link the relevant approved requirements and label independent decisions.

Record input identities: HEAD, dirty-content fingerprint for relevant files, specification identities and amendments, official standard/dependency source dates/versions where used. Revalidate before delivery. Do not silently rewrite the import manifest when local requirements change. Never put secrets or competitor source text in the plan.

For a cross-repository dependency, inspect only authorized consumer public contracts. Identify owner, missing behavior and affected acceptance path. An unverified required dependency blocks readiness. A written request is not acceptance or an implemented dependency. Do not write outside this repository or contact owners unless separately authorized.

## Required reviews

Read [review-protocol.md](references/review-protocol.md). Complete the full sequence:

1. Spawn two distinct reviewers with fresh context (`fork_turns=none`), concurrently when possible. Give each the verbatim user request, confirmed context, repository root, current plan path, approved requirement paths and review contract. Give neither the other's findings, suspected defects, proposed fixes, competitor research or parent conversation history.
2. One traces implementation completeness; the other challenges architecture and verification. **Both** apply `docs/cleanroom.md` and report a provenance verdict alongside technical findings and a requirement/acceptance coverage map. They may inspect approved repository inputs, not edit the plan.
3. Evaluate every finding, apply justified corrections directly to the plan, preserve user decisions and reject unsupported scope expansion. Caller owns edits and final judgment.
4. Spawn a new fresh-context adversarial reviewer after revisions. Supply only the verbatim request, confirmed context, root, revised plan/input paths and review contract. Withhold earlier findings and dispositions. It must attack both technical failure paths and cleanroom violations.
5. Resolve findings and reread the whole plan for contradictions. Keep an implementation-safe review record with reviewer stage, input identities, actual verdicts, accepted/rejected dispositions and limitations; do not copy research or private reasoning. Record it beside the plan as `review-record.md`, not as normative implementation instructions.

Every required stage needs an actual returned review, including an explicit provenance verdict. If fresh context is unavailable or a reviewer cannot inspect inputs, retry once in a fresh session. If that fails, preserve work and report the incomplete stage; do not replace independent review with self-review or call the plan ready. Technical or provenance blockers leave readiness blocked until resolved. An explicit recheck may receive its own earlier findings and fixes; it is not a new independent stage.

## Deliver

Validate structured application context, local links, new/proposed labels, work-package order and acceptance coverage. Confirm source identities still match the reviewed plan or obtain a recheck for material changes. Report plan status, link the overview and first work file, name the next action, and state which reviews actually completed. Keep unsupported runtime/compatibility claims out of the summary. Planning approval is not deployment authorization or a legal cleanroom certification.
