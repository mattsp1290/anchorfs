# Cleanroom engineering policy

This project uses public-behavior-derived requirements and independent engineering choices. It does not claim historically isolated two-team development, legal clearance, or compatibility with a competitor's private protocol. The requirements/bootstrap author encountered public documentation and supplied prose; importing a reviewed specification does not erase that exposure.

## Approved inputs

- The seven-file `docs/spec/` handoff identified by [import-manifest.json](provenance/import-manifest.json), plus explicitly reviewed local amendments.
- AnchorFS implementation, tests, design decisions, user requirements and implementation-safe provenance records.
- Official protocol/OS/platform documentation needed for S3, Linux/FUSE, PostgreSQL, Kubernetes and deployment providers.
- Chosen general-purpose libraries and dependencies, after source origin and license review. Do not use this category to admit competitor clients or implementations under another name.
- Authorized consumer public contracts necessary for integration. Inspect only those boundaries; do not transitively read their competitor research, prompt histories or implementation sources excluded here.

Do not retrieve competitor product pages, source repositories, SDK implementations, binaries, private protocols, traces, copied examples or the external research archive from an implementation/planning/review session. A missing comparator detail is not a reason to bypass this rule. Standard identifiers such as POSIX calls and S3 operations may retain their standard names; independently select AnchorFS routes, schema, package layout and algorithms.

## Missing behavior and updates

First decide whether a gap is an independent product/design choice that can be resolved from requirements and standards. If it needs new competitor evidence, record the behavioral question and affected acceptance path without fetching that evidence. A separate requirements author may research it outside the implementation context and deliver a short independently written specification, provenance category, review outcome and content identity. No automatic external request submission is authorized by this policy.

For imported updates, pin the proposed file allowlist and hashes in an implementation-safe approval record before consuming content. Do not discover approval by opening research-facing reports. Missing/mismatched hashes block the affected import. Keep the original import manifest unchanged; record new approved identities and rationale as a later amendment. Copying a manifest or computing a hash does not itself approve content.

New independent local design decisions can be reviewed in-repository. Record their requirement, rationale, origin category and verification; do not call them competitor behavior. Proposed dependency code must have known source/license before incorporation. Synthetic fixtures and test logic must come from requirements, standards, or authorized data rather than competitor outputs/private traces.

## Required plan and code review checklist

Every reviewer reports **no blocking provenance findings**, **blocking provenance findings**, or **review incomplete**, independently of technical correctness. Identify evidence and limitations; do not provide a legal verdict.

1. Map the user's requirements and each proposed behavior to approved specification sections or explicit independent decisions. Separate repository fact, user decision, proposal, inference and verified execution.
2. Check new code, schemas, tests, examples and docs for unexplained origin, copied expression, vendor-specific names/layout, private interfaces and unsupported parity/performance claims. Distinguish standard names from vendor-specific expression.
3. Check dependency/source licenses and provenance records; missing origin is a blocker for incorporation, not automatically permission to rewrite the same material.
4. Verify source updates retain the original baseline and have explicit approval records. A hash match proves identity, not originality or legal suitability.
5. Check fresh reviewer context and the approved-input packet. No raw research, prior reviewer conclusions or inherited research history in independent reviews. Later rechecks may see their own findings and fixes and must be labeled rechecks.
6. Require independently authored acceptance evidence. Plans, mocks and old review verdicts are not proof that code works or is independently derived.

Reviews must also check normal technical correctness: transaction/fencing invariants, failure recovery, quotas, permissions, provider differences and lifecycle limits where affected. Passing provenance review never substitutes for technical review or tests.

## Exposure response

Stop affected work if prohibited material or unknown source origin is encountered. Record the source category, artifact identity, access boundary and potentially affected artifacts without copying the prohibited content. Preserve evidence under an appropriate separate audit boundary; do not commit raw material into AnchorFS. Ask the requirements owner to decide whether an approved replacement specification, independent reimplementation context or additional review is needed. Unaffected work may proceed. Never claim that deletion, paraphrase or a fresh prompt erases prior exposure.

Fresh sessions and directory boundaries are procedural controls, not an access-control system. If a required fresh reviewer cannot be created, leave that stage incomplete. An agent already exposed to excluded material must disclose that limitation and cannot label itself a fresh independent implementer/reviewer merely by rereading these rules.
