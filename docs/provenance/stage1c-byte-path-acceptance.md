# Stage 1c executable acceptance map

This maps the approved plan to executable tests. Command outcomes, timing,
review verdicts and CI evidence live in [the evidence record](stage1c-byte-path-review.md).
A mapping is not itself proof of a passing test. The [implementation amendments](stage1c-implementation-amendments.md)
record the 156-column gate, dependency-edge exception and BY20 history oracle.
Imported specifications and their original import manifest are unchanged.

All byte tests are registered in
[`tests/bytes.rs`](../../crates/anchorfs-conformance/tests/bytes.rs), using fresh
PostgreSQL databases and distinct prefixes in the pinned disposable MinIO.

| Plan acceptance | Executable coverage |
|---|---|
| BY1: complete two-chunk sparse write, native cross-session read, generation | `reads::by1_two_chunk_sparse_write_is_read_identically_by_another_session`; `reads::by1_read_uses_one_manifest_while_replace_publishes_a_new_generation` |
| BY1b/BY1c: appends and overlapping writes | `recovery::by1b_appends_follow_current_size_including_a_far_write`; `concurrency::by1c_overlapping_writers_preserve_unrelated_bytes_after_retry` (sequential and concurrent sessions) |
| BY2: key binds payload, offset, path and size | `writes::by2_payload_and_offset_bind_the_key_and_empty_writes_admit_nothing`; `writes::by2_replace_paths_and_truncate_sizes_bind_receipts` |
| BY3/BY3d: lost response and replay precedence | `writes::by3_lost_write_response_replays_before_handle_and_quota_validation`; `writes::by3_lost_positional_write_response_replays_the_original_offset`; `replace::by3d_lost_replace_responses_resolve_for_nonempty_and_empty_files`; `truncate::by3_truncate_response_abandonment_resolves_by_receipt_and_replay` |
| BY3b: expiry and B11 | `fencing::by3b_expired_byte_receipts_fence_sessions_but_failed_receipts_do_not`; control-plane `expired_receipt_followup_recovers_after_a_crash` and `cp4_cp12_receipts_replay_mismatch_visibility_and_tombstones`; persistence `expired_receipt_tombstone_then_session_fence_once` (prior-state matrix and exactly-once session epoch change) |
| BY3c: actual COMMIT loss and definite rejection | `recovery::by3c_connection_loss_during_manifest_commit_preserves_atomicity`; `recovery::by3c_definite_commit_rejection_finalizes_without_publishing` |
| BY4: pending same key | `recovery::by4_pending_same_key_returns_the_owned_operation_then_replays`; BY20 parked duplicates |
| BY5/BY5b/BY5d: crash, same-key fence and quota-pressure recovery | `recovery::by5_crash_is_durably_fenced_by_same_key_or_quota_pressure`; BY20 pressure probe; persistence reservation/receipt fence schedules |
| BY5c: router restart over pending state | `recovery::by5c_new_router_recovers_a_crashed_pending_upload` |
| BY6: four deterministic provider faults | `backend_faults::by6_proxy_faults_preserve_old_bytes_and_conservative_charges`; `writes::by5_upload_failure_retains_charges_and_a_terminal_failed_receipt` |
| BY7: workspace/principal/logical quotas, late race | Both `quotas::by7_*` tests; BY6 physical charges for failed uploads |
| BY7b: deployment headroom and workspace isolation | `quotas::by7b_deployment_staging_is_shared_and_by13_pins_do_not_cross_workspaces` |
| BY7c: exact materialized reservation | `recovery::by7c_reservations_charge_exact_materialized_lengths` |
| BY8: bounded retry, atomic staging swap, metadata-only change, strict replace | `concurrency::by8_generation_retry_is_bounded_and_ignores_metadata_only_changes`; `writes::by8_concurrent_appends_swap_reservations_at_the_staging_limit`; `replace::by8_conditional_replace_rejects_data_and_metadata_changes_without_retry` |
| BY9: credential/session/workspace/handle/reservation/workflow fences | All `fencing::by9_*` tests (including expiry during a post-validation quota wait); `writes::by9_publication_rechecks_credential_epoch_and_handle_lifetime`; control-plane credential/session cascade tests |
| BY10/BY10b/BY11: truncate, pins, unlink-open and zeros | All `truncate::by10_*`, `truncate::by10b_*`, `truncate::by11_*`; `writes::by1_writes_append_and_sparse_truncate_publish_complete_manifests`; BY8 racing truncate; byte model |
| BY12: whole-object integrity and independent files | `reads::by12_reads_verify_whole_objects_including_discarded_tails` |
| BY13: pin generation, isolation, limits, lease admission and expiry | `reads::by13_pins_fix_data_generation_and_are_session_bound_bounded_and_expiring`; `access::by13_real_pin_ttl_and_by15_expired_fsync_use_database_time`; BY7b workspace isolation; byte model publication between pin and pinned read |
| BY14/BY14b: streamed replace, shape, malformed body, zero length and stalled body | All `replace::by14_*` and `replace::by14b_*`; `access::by14_replace_rejects_directory_symlink_and_read_role_before_admission` |
| BY15/BY16: fsync and permissions | `writes::by1_writes_append_and_sparse_truncate_publish_complete_manifests`; both relevant `access` tests (receipt-free fsync, TTL, read/write handle access, admin replacement of mode-000 file) |
| BY17: no transaction over external waits | `concurrency::by17_pipeline_fault_waits_and_delayed_backend_io_hold_no_transaction`; read snapshot/publication schedule |
| BY18: 34 implemented / 4 workflow stubs | Conformance `service` route sweep, header/body/shape validation and service deadline tests |
| BY19: independent-file timing and lost concurrent appends | Both `concurrency::by19*` tests |
| BY20: randomized concurrent histories and deadlock oracle | `stress::by20_randomized_concurrency_has_no_deadlocks_lost_publications_or_pending_receipts` (D-C4 describes publication/final-state oracle) |
| BY21: four-connection pool pressure | `pool::by21_pool_saturation_keeps_an_ambiguous_receipt_recoverable` |
| Sequential model | `bytes_model::byte_model_sequences`: 32 default sequences, 1–12 operations; dense per-file bytes and inode/data generations; stat/native reads and invariant/bucket checks after every generated operation |
| Pure geometry property | `chunking_props::sparse_chunk_plans_match_dense_bytes`: 256 default cases, independent dense oracle |
| Persistence paths and lock schedules | `anchorfs-persistence/tests/bytes_tx.rs` and `tests/bytes_tx/*`, plus namespace handle/authority schedules |
| Actual process restart and geometry refusal | Service binary `restart::serve_restart_preserves_bytes`, `serve_refuses_changed_chunk_size`, `config_byte_path_bounds` |
| Regression/structural/provenance gates | Workspace tests, OpenAPI snapshot, `design_coverage` (156 columns), dependency provenance, fmt, clippy, deny, unchanged import hashes |

The overview's eleven success criteria map respectively to BY1/BY10/model;
BY3/BY3c/BY3d/BY4/BY19; binary restart/BY5c; BY2/BY3b; BY5/BY6/BY7;
BY8/BY9; BY10/BY11/model; SQL/deployment/bucket oracles; BY12; BY13; and
BY1c/BY19/BY20. Checks during a parked or deliberately corrupt state assert
that state's intended charges/visibility; final reconciliation is performed
once the synthetic corruption is restored and concurrent tasks have quiesced.

This milestone does not prove Linux mounts, Spaces/AWS provider behavior,
container/storage crash durability, collection, a sweeper or historical legal
isolation. Completed reviewer and hosted-CI results are recorded in the evidence record.
