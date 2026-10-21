# Stage 2a executable acceptance map

Approved plan: `.agents/plans/stage2a-workflow-leases/`. All fixtures are synthetic
and use disposable PostgreSQL/MinIO. This map identifies executable checks;
actual results and review stages belong in the evidence record.

| Criterion | Executable coverage |
|---|---|
| 1: lifecycle, owner rules, receipt metadata/replay | namespace `lease_lifecycle_receipts_and_replay`; persistence `acquire_release_takeover_transitions`, `renew_release_require_owner_and_live_proof`, `non_owner_renew_release_on_dead_or_free_lease` |
| 2: expiry/takeover and real TTL | namespace `expired_lease_requires_takeover`, `lease_expires_by_timer`; persistence `acquire_rejects_live_and_dead_leases` |
| 3: T19 acquire fences upload; binding permits publication | bytes `t19_real_acquire_fences_inflight_upload_and_bound_worker_publishes` |
| 4: takeover fences old uploads; successor publishes | bytes `takeover_fences_old_lease_upload_and_successor_publishes` |
| 5: spec 07 revoked worker cannot reenter | bytes `spec07_revoked_worker_cannot_reenter_while_host_and_successor_continue`; namespace `lease_routes_reject_workers_read_hosts_and_bound_actors` |
| 6: fail closed on release/takeover, explicit rebind; expiry reads | namespace `bound_sessions_fail_closed_until_rebind`, `expired_lease_requires_takeover`; both byte fencing scenarios; persistence `bind_then_release_leaves_bound_session_fenced`, `release_fences_a_session_bound_after_lock_wait`, `worker_with_token_and_no_lease_is_forbidden`, `any_host_may_rebind_a_session_bound_to_another_hosts_lease` |
| 7: visible status, secrets omitted | namespace `workspace_reports_lease_status`; lifecycle receipt queries; service `workflow_grant_strips_token_from_stored_value`, `request_debug_redacts_lease_tokens`; persistence `workflow_debug_redacts_token_and_hash` |
| 8: lock order and concurrency | namespace `concurrent_acquires_have_one_winner`, `lease_routes_reject_workers_read_hosts_and_bound_actors`; persistence `concurrent_acquires_serialize_without_deadlock`, `unauthenticated_acquire_does_not_block_readers`, `lease_wait_past_deadline_does_not_commit`, `acquire_under_publication_load`, `invariant_detects_bound_epoch_above_workspace`, `worker_read_role_and_bound_sessions_cannot_hold_leases` |
| 9: contract closure | conformance `service` (38/0), `design_coverage` (156), `provenance`; service `openapi_snapshot`, config binary bounds; fmt/clippy/deny; unchanged lockfile/import identities |

The persistence suite includes fifteen new tests: fourteen schedule/authority
checks (including the publication-load observation) and the documentation-only
`lock_upgrade_from_share_is_documented`. Existing workflow predicate tests
remain intact. `acquire_under_publication_load` observes one schedule; it does
not prove starvation freedom or bounded acquisition latency.

Implementation correction to the plan's repository inference: existing
`HandleRequest::mutation_gate` gates writable opens as mutations even with
create:false. The new byte tests use read-only opens for readers and an
unbound worker's busy-path probe; lease holders use proof on writable opens.
No existing handle behavior was changed. The unbound probe fails at the lease
gate before checking its read-only handle's write access; ordinary writable
handles are exercised before acquisition and after host rebind.
