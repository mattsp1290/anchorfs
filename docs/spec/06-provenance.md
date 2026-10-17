# Engineering provenance and cleanroom boundary

This handoff uses public behavior as input and independently chosen architecture as output. It is a documentation-based independent implementation process, not a claim of a legally certified cleanroom or historical two-team isolation.

## Implementer input boundary

Use `spec/` as the implementation brief. Research evidence and review reports are for provenance/audit; the implementer does not need competitor materials. Public protocol standards and official platform/library documentation may be consulted directly. Do not inspect Archil source repositories, SDK/native-client implementation, binaries, internal traces, private APIs, decompiled code, leaked material or credentials to fill gaps.

If a new behavior is missing, specify it independently and test it, or ask the requirements author for a behavioral clarification. Do not reverse-engineer private protocol compatibility. Product name, package names, CLI flags, endpoint routes, accounts, policy identifiers and branding should be independently selected. Standard names such as S3 operation identifiers and POSIX calls retain their standard meaning.

## Evidence discipline

Separate three categories in future changes: user requirements, publicly documented observable behavior, and independently selected engineering decisions. Preserve source URLs, access dates, and a short paraphrase for relevant factual claims. Do not paste competitor prose, diagrams, code samples or API SDK types into implementation artifacts. Unknowns remain unknown until evidence exists.

Record dependency licenses and source origins before incorporating third-party code; open-source availability is not permission to ignore license terms. Use only synthetic fixtures or user-authorized data. Do not test against a vendor account or system without task authorization.

## Separation and review limits

The requirements author encountered public documentation, including displayed SDK examples, and user-supplied post prose; no competitor implementation code or binaries were inspected. Reviewers share access to this repository, so directory separation is a workflow aid rather than a technical access barrier. If a stricter organizational process is required, export only the approved `spec/` bundle to a fresh implementation context and preserve this audit material separately. Do not represent that export as erasing prior exposure by a person or agent.

The dedicated cleanroom review assesses document provenance and contamination risks. It does not determine patent clearance, trademark rights, contract restrictions, or legal non-infringement. Those determinations are outside this engineering review.

## Release provenance record

For every incorporated module, retain author/tool provenance, dependency origin/license, requirements IDs and tests. Reject unattributed competitor-derived code. Record independent design choices and their rationale, especially transaction ordering, chunk format, failure model and concurrency semantics. Never describe choices here as Archil's implementation.
