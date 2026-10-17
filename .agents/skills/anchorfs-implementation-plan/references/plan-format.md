# Plan Format and Quality Bar

Write for a coding agent that did not participate in planning. The agent must be able to implement the change without reconstructing the architecture or guessing what “done” means.

## Required document set

Use numeric prefixes so a filesystem listing preserves reading order. Scale the middle files to the change, but always create a series rather than one monolithic document.

```text
<plan-name>/
├── 00-overview.md
├── 01-<first-domain-or-readiness-gate>.md
├── 02-<next-work-area>.md
├── ...
└── NN-execution-handoff.md
```

`00-overview.md` must contain:

- planning status and a clear statement that implementation has not occurred;
- a user-confirmed application-context record in the exact structure below;
- the repository-inferred change type and affected areas;
- the requested outcome and measurable success criteria;
- scope, non-goals, and constraints;
- repository-grounded findings that drive the design;
- key decisions with reasons and rejected alternatives when they matter;
- the clearest relevant change model, such as target architecture, before/after control or data flow, configuration transition, or dependency transition;
- risks, assumptions, unresolved decisions, and stop/go gates;
- a document map with one-line purposes;
- when external requests exist, their canonical locations, owners, demonstrated/prospective consumers, affected work packages, blocking status, and exact unblock evidence, including any proposed new repository.

## Application-context record

Put exactly one fenced `json` block under an `## Application context` heading in `00-overview.md`:

```json
{
  "application_context": {
    "has_active_users": true,
    "backward_compatibility_required": true,
    "feature_flags": "appropriate",
    "confirmation_digest": "<lowercase-sha256>",
    "confirmed_at": "<RFC3339 timestamp>"
  }
}
```

Use booleans for the first two fields. `feature_flags` must be one of `appropriate`, `not-appropriate`, `decide-per-pr`, or `not-applicable`; use `not-applicable` only when both booleans are false. Compute `confirmation_digest` as the lowercase SHA-256 of this UTF-8 byte sequence, using lowercase `true` or `false` and literal NUL separators:

```text
implementation-plan-application-context-v1\0<has_active_users>\0<backward_compatibility_required>\0<feature_flags>\0<confirmed_at>
```

The timestamp records when the user supplied or corrected the answers. Text immediately outside the JSON block may explain implications, but it must not contradict the structured values. If either required boolean is unanswered, omit the JSON block, set `Status: Blocked`, and name the missing answer and owner. A plan without a valid block is not ready for implementation.

When `feature_flags` is `decide-per-pr`, mark every behavior-changing work package with a feature-flag decision gate. Otherwise apply the recorded strategy directly to relevant work packages. Describe concrete compatibility surfaces—APIs, stored data, configuration, and workflows—in the affected work files rather than relying only on the overview boolean.

Classify each unresolved decision as blocking or non-blocking. A plan with an unresolved blocking decision must show `Status: Blocked`, the decision owner, and the exact action that unblocks implementation. Do not describe that plan as implementation-ready.

Create middle files around cohesive implementation work. Prefer architectural boundaries, dependency boundaries, or verification gates over arbitrary file-count targets.

`NN-execution-handoff.md` must contain:

- dependency-ordered work packages;
- the files, symbols, interfaces, or schemas each package changes;
- prerequisites and parallelization constraints;
- verification commands or procedures for each package;
- integration and regression gates;
- a final definition of done;
- deferred work and follow-up items.

## Work-package content

For each meaningful work package, specify:

1. Goal and prerequisite state.
2. Repository evidence and relevant existing patterns.
3. Exact change surface: paths, symbols, interfaces, configuration, and migrations. Label files and symbols that do not exist yet as `new` or `proposed`.
4. Intended behavior, invariants, error paths, compatibility requirements, and lifecycle concerns.
5. Tests and observable acceptance criteria.
6. Dependencies, risks, edge cases, and explicit exclusions.

Use small pseudocode, tables, trees, or state diagrams when they remove ambiguity. Do not write large implementation-ready code blocks that will become stale before coding starts.

## Grounding rules

- Inspect before asserting. Existing repository references must resolve. Label proposed paths and symbols as `new` or `proposed`, and anchor each one to an existing parent path or insertion point.
- Mark conceptual API names as conceptual. Do not present invented names as an existing contract.
- Separate verified facts from proposals and assumptions.
- Record important dirty-worktree constraints when they affect implementation. Do not treat unrelated user changes as plan work.
- Preserve user decisions. Do not expand the product or authorize external changes through the plan.
- Name the owner and resolution point for any decision that the implementer cannot make safely.
- Put risky or irreversible actions behind explicit readiness and rollback gates.
- Never include credential values, tokens, passwords, private keys, session identifiers, or unredacted connection strings. Refer to secret variable names and redacted examples only.

## Writing rules

- Use one stable term for each component or concept.
- Prefer active voice and direct verbs.
- Put one instruction or acceptance condition in each sentence or list item.
- Keep uncertainty when the evidence is uncertain. Never convert “may” into a fact.
- Avoid vague verbs such as “handle,” “support,” “update,” or “ensure” unless the following text defines the observable result.
- Avoid prose that only says to follow best practices, add tests, or update documentation. Name the behavior, test level, and acceptance condition.
- Keep each file internally useful but avoid copying whole sections across files. Use links for shared decisions.
