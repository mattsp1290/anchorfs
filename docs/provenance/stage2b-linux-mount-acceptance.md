# Stage 2b executable acceptance map (verification in progress)

The test names below identify the intended evidence. A named test is not proof
of a pass; actual execution and remaining gates are recorded in
[the evidence record](stage2b-linux-mount-review.md).

| Criterion | Executable evidence |
|---|---|
| 1 API/mount byte visibility | `visibility::mount_write_fsync_release_visible_to_api_and_back` |
| 2 two mount processes, one kernel | `visibility::two_mount_processes_share_committed_files` |
| 3 namespace and handle semantics | `visibility::mount_lookup_stat_readdir_match_api`, `handles::rename_over_open_and_unlink_open_keep_handle`, `visibility::reader_sees_remote_write_on_next_read`, `handles::hard_link_names_survive_unlink_of_one`, `lifecycle::replaced_ancestor_directory_is_estale` |
| 4 fencing, renewal, revocation and shutdown | `fencing::paused_mount_past_expiry_is_fenced`, `fencing::lease_bound_mount_publishes_and_ordinary_mount_is_busy`, `fencing::takeover_fences_bound_mount`, `fencing::expired_lease_latches_bound_mount_without_takeover`, `fencing::bound_fence_rejects_inflight_read_result`, `fencing::renewal_keeps_handles_and_session_alive`, `fencing::sigstop_paused_mount_is_fenced_on_resume`, `fencing::revocation_fences_mount`, `fencing::ordinary_mount_survives_lease_race`, `lifecycle::sigterm_with_open_files_keeps_serving` |
| 5 unsupported operations fail explicitly | `unsupported::unsupported_operations_are_explicit` |
| 6 Git under a lease | `git_smoke::git_smoke_under_lease` |
| 7 explicit times/mode/truncate | `handles::setattr_times_mode_truncate`, conformance `times::set_attributes_times_owner_admin_and_forbidden`, `times::set_attributes_times_replay_and_lease_gate` |
| 8 partial writes, append and capacity | `handles::partial_write_reports_committed_bytes`, `handles::append_handles_use_server_eof`, portable `handles::tests::*` |
| 9 typed client, receipts and contract closure | client unit tests; conformance `client`, `service`, `design_coverage`, `provenance`; service `openapi_snapshot`/`binary`; `cargo deny` |
| 10 host and hosted gates | serial Linux mount suite and full workspace suite in `docs/ops/linux-host-verification.md`; all five CI jobs on final PR revision |

The kernel tests live in `crates/anchorfs-mount/tests/linux.rs`; the fixtures
and raw syscalls are Linux-only. Portable mount, client and CLI tests also run
on macOS. `cli::cli_exit_codes_and_secret_sources` checks startup errors and
secret sources; Linux unit `platform::linux::tests::fuse_config_and_option_rendering`
checks the strict mount options and allow-other ACL selection.
