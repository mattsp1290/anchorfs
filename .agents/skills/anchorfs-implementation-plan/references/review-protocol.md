# Review contract

Read repository-relative `docs/cleanroom.md` before inspection. Every review covers technical correctness and provenance; neither verdict substitutes for the other.

## Input packet

Each independent reviewer receives fresh context, verbatim user request, explicit application-context answers, repository root, plan path, approved requirement paths and this contract. Permit relevant approved repository code/history inspection. Exclude competitor research/private sources, prior reviews/dispositions, suspected defects, solutions and parent conversation history. Do not edit files or implement. If approved context is insufficient, report the missing input rather than opening prohibited materials.

## Findings and coverage

For each material finding return:

- Severity: Critical, Important or Minor.
- Plan location and requirement affected.
- Repository evidence or exact missing evidence/gate.
- Failure mode if a competent agent follows the plan.
- Concrete correction within the user's scope.

Omit compliments and style preferences. If no material technical findings, say `No material findings`. Separately map explicit user outcomes, constraints, non-goals and compatibility promises to plan locations and observable acceptance. Check application_context against actual user answers; never infer or rewrite them.

Report provenance verdict: `no blocking provenance findings`, `blocking provenance findings`, or `review incomplete`. Give evidence and limits for input origin, independent design, copying/vendor-specific expression, dependency licenses, new-source approval, test-fixture provenance, and unsupported parity/performance claims. Unknown source origin blocks the affected incorporation. Do not assert legal clearance or historical isolation.

## Roles and sequencing

Initial reviewer A traces implementation completeness: change surfaces, control/data flow, dependencies, lifecycle/error paths, sequencing and acceptance.

Initial reviewer B challenges architecture and verification: repository fit, public boundaries, isolation, durability, recovery/rollback, provider assumptions, test layers and false confidence from mocks.

Keep initial reviewers independent. The parent evaluates and applies accepted findings, then a new reviewer attacks the revised plan with this stance:

> Assume a competent coding agent follows the plan exactly and it still fails or violates the input boundary. Find the strongest ways that happens.

The adversarial reviewer gets no earlier findings or dispositions. It ranks unverified assumptions, contradictions, incomplete fencing/cleanup/ownership, unsafe dependencies, tests that pass despite broken behavior, and contamination routes. Same finding, coverage and provenance contract applies.

## Closure

The parent owns edits. Missing capability, unusable output or non-fresh reviewer context is incomplete review: one fresh retry, then stop that stage if still unavailable. Accepted material changes must be reflected in the normative plan and identity record. Technical and provenance blockers must both be closed before ready status. If a human decision is needed, state its owner and exact question instead of inventing an answer. Persist only concise findings/dispositions and evidence, not hidden reasoning or prohibited content.
