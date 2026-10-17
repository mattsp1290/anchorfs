# Harness and tool-worker lifecycle

This independently specified integration contract makes the storage service usable by a harness that launches separate, short-lived tool workers. It does not require a production scheduler, sandbox platform, or remote command API in the initial storage release.

## Ownership and persistence boundaries

The host owns authenticated user/tenant identity, workspace authorization, agent sessions, dispatch and execution policy. The storage service owns committed file contents and metadata. An executor owns process isolation, runtime image, CPU/memory limits, network policy, deadlines and worker termination. None of those security responsibilities is supplied merely by sharing a filesystem.

Workspace identity and lifetime MUST be independent of worker, pod, tool-call and inference-request lifetimes. A worker exit, scale-down, idle harness or tool timeout must not delete its workspace. Explicit retention/deletion policy belongs to the host. The harness may read committed files through the native API while workers access the same namespace through mounts; there is no per-worker copy to reconcile.

Only committed filesystem state persists. Process memory, background processes, open descriptors, environment variables and modifications to an ephemeral container root do not survive worker replacement. Dependencies installed inside the workspace persist as files; OS packages installed outside it require an image or separate explicitly managed persistence. Tool callers must not assume that a later worker inherits the previous process environment.

## Tool-call flow

1. The harness authorizes the workspace and records a durable tool-call identity in its existing session store. Dispatch uses an idempotent job identity separate from storage operation IDs.
2. An executor provisions a worker with a verified runtime image and explicit resource, network and isolation policy. The worker receives a distinct, revocable execution capability with bounded lifetime, workspace, role and session/epoch binding, never a reusable host/workspace-administrator credential or backing-bucket credentials. It cannot mint new sessions, rebind expired mounts or acquire arbitrary workflow leases; only host/executor authority can do that. Revocation closes existing access and prevents fresh-session reentry without revoking the harness or unrelated workers. Resolve secrets at the host/executor boundary; do not persist them in shared workspace files by default.
3. The worker attaches to the stable workspace. For a multi-step exclusive workflow, bind the mount to the workspace lease epoch before starting the command. Different-file operations can use the ordinary shared mode when no exclusive workflow is active.
4. The worker runs the tool, checks filesystem errors and completes required write/fsync barriers before reporting filesystem success. Synchronous acknowledged writes are already durable under the storage contract. A failed command can leave committed partial changes: execution failure is not a rollback.
5. The host records the tool outcome in its session store and tears down worker access. If interruption/timeout leaves execution uncertain, revoke or fence the old mount identity and verify termination of the entire execution group/container, including descendants, before admitting replacement work under an exclusive lease. If termination cannot be established, report unresolved execution state and require an explicit host recovery policy rather than treating storage fencing as proof of termination. Fencing filesystem writes does not stop external network side effects.
6. A different worker or the harness reads the committed workspace using the same authority. The storage layer remains available without the original worker. This does not imply that database/storage servers themselves scale to zero or that a particular scheduler incurs no idle cost.

## Retry and cancellation limits

The host session transaction and the storage metadata transaction are independent. The executor must expose durable status lookup for ambiguous dispatch/completion. Do not promise exactly-once shell execution from storage mutation receipts. Arbitrary commands can have external effects and contain many writes; retrying a command after a lost response may repeat those effects. Reconcile known job status or report an indeterminate outcome instead of automatically rerunning.

A worker may disappear after file writes commit but before the tool result is recorded. The files remain committed; the harness must distinguish unknown execution outcome from lost filesystem data. Cancellation, credential revocation and lease takeover must follow the commit-time fencing predicates in the storage specification. No automatic whole-workspace rollback is supplied.

## Acceptance cases for the storage release

- Harness writes via API; worker A reads identical bytes through a mount. Worker A writes and completes its barrier; a subsequent harness API read observes the committed version.
- Terminate worker A after successful writes. Worker B on another node mounts the same workspace and reads those files. No copy-back/export step is required.
- Keep the harness session idle with no tool worker running, then attach a new worker. Files remain available; the storage metadata and object services remain in operation.
- Lose the completion response after a worker commits files. Demonstrate an uncertain tool outcome without blindly redispatching the command or losing committed files.
- In a storage-only fencing test, deliberately quiesce an uploading worker with external effects disabled, revoke/fence its mount, and start a successor under the coordinated workflow. Production replacement separately requires the execution-group termination verification above. The old worker cannot later publish. A command that made earlier committed changes may leave them visible.
- Attempt new session registration, expired-mount rebinding and lease acquisition with the revoked worker capability: all fail, while the harness and successor retain their authorized access.
- Destroy the ephemeral worker root and process state. Workspace files survive; document that packages/state outside the mount do not.

These cases can use a test orchestrator and disposable Kubernetes pods. They do not certify sandbox security, compute-cost improvements or production scheduler semantics. A later Lambda host can use this same boundary via API calls and remote workers; no custom FUSE mount inside Lambda is assumed.
