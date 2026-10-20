# Stage 1c implementation amendments — 2026-10-20

These are independent local engineering details needed to implement the approved
behavior. They do not change product scope or imported specifications. The
requested fresh-context review gauntlet must assess them with the implementation;
this file records decisions, not a reviewer verdict or legal clearance.

## D-C1 — Durable expiry follow-up eligibility

W35 requires repeatable post-tombstone session revocation for pending/committed
receipts, excluding failed receipts, after result is cleared. The original
proposed migration retained no durable distinction between those tombstones.
Migration 0009 adds operations.expiry_fences_session, boolean, default false;
tombstoning sets it from the locked prior state. Later encounters and the
idempotent session fence check that flag. The persistent-column gate is **156**,
superseding 155 in WP1/WP4. Source: approved W35 and independent crash-recovery
reasoning. Tests cover repeated encounters for all three prior states, crash
before the follow-up, one-time epoch increment and failed-receipt exclusion.
This decision was recorded in provenance at the foundation checkpoint; this
amendment consolidates that record for delivery review.

## D-C2 — Typed HTTP body-limit error classification

The streamed length-overrun test exposed a distinction required by W4/W14:
tower-http limits the body to the declared Content-Length, so an overrun arrives
as a typed http-body-util LengthLimitError, whereas transport errors must remain
503 dependency_unavailable. Error-message matching would be fragile.

Add a direct service dependency and exact workspace pin for http-body-util
0.1.5, already in Cargo.lock. cargo info for that exact version and its registry
LICENSE were checked before incorporation: MIT, official general-purpose source
https://github.com/hyperium/http-body, registry checksum
23169fe34a5fbcdd3f3862e78fb9b6fccd5f02a6dc6f732547005d45631ce71c.
Its provenance row was added before using the type. No competitor source is
involved. A metadata lookup initially downloaded 0.1.3; it was not incorporated,
and the locked 0.1.5 identity/license were then verified.

Cargo.lock changes for **two existing runtime edges** in anchorfs-service:
futures-core and http-body-util. The package set and all package versions remain
unchanged. This supersedes the plan's one-edge-only gate. The source/license
gate was satisfied before incorporation; this consolidated planning amendment
was written after the local error-classification test, a sequencing deviation
recorded for review rather than represented as prior approval.

## D-C3 — Empty replacement uses the data runner's atomic transaction

W26's empty replacement has no reservation and never commits visible pending
state. It uses one metadata transaction inside the data runner, with the same
receipt and publication checks as a control-runner mutation. This preserves the
atomic admission handshake and the data deadline without nesting two runners.
Body framing/digest validation occurs before that transaction, after receipt
peek. A malformed empty body commits a terminal failed receipt in that single
transaction; a valid body publishes the empty manifest and receipt together.
No transaction spans body polling or backend I/O, and no object is uploaded.
Post-commit lost-response tests cover both empty and nonempty replacements.

The stream owns one application chunk buffer, one pending incoming body frame,
the running hasher and bounded manifest descriptors. Incoming transport frames
are not rebuffered as a whole file; their allocation is owned by the body
transport. This qualifies the plan's one-chunk memory shorthand explicitly.
