//! Phases 5-6: revocation fencing, the bound successor under a workflow lease, the negative
//! checks with the revoked bearer, and the ephemeral-root case. The lease spans both phases.
use crate::{
    api::status_of,
    cases::Proof,
    harness::{Connected, Slot, wait_for},
    journey::Principals,
    pods::{WORKSPACE_PATH, WorkerPod},
};
use anchorfs_client::{
    Client,
    auth::{SessionAuthority, WorkflowAuthority},
    error::ClientError,
    retry::IdempotencyKey,
};
use anchorfs_core::ErrorCode;
use serde_json::json;

impl Proof {
    /// Revoke first (D19), then lease and bound successor. Returns the live successor pod D.
    pub fn case5_6_fencing_then_successor(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        client: &Client,
        pod_c: WorkerPod,
    ) -> (WorkerPod, WorkflowAuthority) {
        self.enter("5-case5-6-fencing-then-successor");
        let fence_log = "proof/fence.log";
        let fence_path = format!("{WORKSPACE_PATH}/{fence_log}");

        // (a) a worker-side append loop, confirmed to grow through the API.
        conn.pods.start_tick_loop(&pod_c, &fence_path);
        let first = wait_for("fence.log to appear", || {
            conn.block_on(client.stat(Some(fence_log)))
                .ok()
                .map(|a| a.size)
        });
        let second = wait_for("fence.log to grow", || {
            let size = conn.block_on(client.stat(Some(fence_log))).ok()?.size;
            (size > first).then_some(size)
        });
        self.evidence
            .phase("5-case5-6", "growth_samples", json!([first, second]));

        // (b) revoke worker-3 first, then prove the mount latches with ESTALE.
        let api = &conn.api;
        api.revoke_credential(&auth.admin_authority(), auth.workers[2].id());
        // The revocation committed when the DELETE returned; the boundary is taken now, not
        // after the mount latches, so a write accepted in between cannot be absorbed.
        let (size_at_revocation, generation_at_revocation) = conn.stat(client, fence_log);
        wait_for("the \"mount fenced\" event (unauthenticated)", || {
            conn.mount_log_has(&pod_c, "mount fenced", Some("unauthenticated"))
                .then_some(())
        });
        let at_fence = conn.stat(client, fence_log);
        assert_eq!(
            at_fence,
            (size_at_revocation, generation_at_revocation),
            "bytes published between the revocation commit and the fence latch"
        );
        let late = conn
            .pods
            .exec(&pod_c, &format!("echo late >> {fence_path}"));
        assert!(
            !late.success() && !late.timed_out,
            "late write after revocation must fail, got exit {:?}",
            late.code
        );
        let late_text = late.stderr.trim().to_owned();
        assert!(
            late_text.contains("Stale file handle"),
            "late write must fail with ESTALE (Stale file handle), got: {late_text}"
        );
        assert!(
            !late_text.contains("busy"),
            "EBUSY here would mean lease-busy, which has not been introduced yet: {late_text}"
        );
        self.evidence.phase(
            "5-case5-6",
            "revocation",
            json!({
                "size_at_revocation": size_at_revocation,
                "generation_at_revocation": generation_at_revocation,
                "size_at_fence": at_fence.0,
                "late_write_exit_code": late.code,
                "late_write_error_estale": late_text,
            }),
        );

        // (c) lease, bound session for worker-2's credential, successor pod D.
        let host = auth.host_authority();
        let lease = api.acquire_lease(&host, auth.workspace);
        let bound = api.create_bound_session(
            &auth.host.authority(None),
            auth.workspace,
            auth.workers[1].id(),
            lease.token.expose(),
        );
        let workflow_secret = format!("anchorfs-workflow-{}", &self.run[..8]);
        self.kubectl.create_secret(
            &workflow_secret,
            "token",
            lease.token.expose(),
            &[("anchorfs.proof/run", self.run.as_str())],
        );
        let pod_d = self.create_worker(
            conn,
            auth,
            Slot {
                tag: "d",
                node_index: 1,
                worker_index: 1,
            },
            &bound,
            Some((workflow_secret.as_str(), lease.epoch)),
        );
        conn.exec_ok(
            &pod_d,
            &format!(
                "echo successor | dd of={WORKSPACE_PATH}/proof/successor.txt conv=fsync status=none"
            ),
        );
        let successor = conn.read_file(client, "proof/successor.txt");
        assert_eq!(successor, b"successor\n", "successor.txt through the API");
        // Two distinct observations while the lease is held. First, pod C is already fenced,
        // so a write through its mount only re-checks the ESTALE latch (not lease-busy).
        let after_lease = conn
            .pods
            .exec(&pod_c, &format!("echo again >> {fence_path}"));
        self.evidence.observe(
            "fenced_mount_write_after_lease",
            json!({"exit_code": after_lease.code, "stderr": after_lease.stderr.trim()}),
        );
        // Second, the real lease-busy phenomenon: an ordinary (unbound) session mutating while
        // the lease is held is refused with 409 workspace_busy (EBUSY at a mount).
        let busy = conn.block_on(client.create_directory(
            "proof/busy-probe",
            0o755,
            &IdempotencyKey::new(),
        ));
        match busy {
            Err(ClientError::Api(failure)) => {
                assert_eq!(
                    (failure.status, failure.code.kind()),
                    (409, Some(ErrorCode::WorkspaceBusy)),
                    "ordinary-session mutation under an active lease: {failure:?}"
                );
                self.evidence.observe(
                    "ordinary_session_mutation_while_leased",
                    json!({"status": failure.status, "code": failure.code.as_str()}),
                );
            }
            Err(error) => panic!(
                "ordinary-session mutation under an active lease failed unexpectedly: {error}"
            ),
            Ok(_) => panic!("ordinary-session mutation succeeded while the workflow lease is held"),
        }

        // (d) negatives with the revoked worker-3 bearer; the host and worker-2 keep working.
        let revoked = &auth.workers[2];
        let revoked_session = &auth.sessions[2];
        let as_revoked = |session: Option<&SessionAuthority>| {
            conn.client_for(auth.workspace, revoked.authority(session))
        };
        let revoked_with_session = revoked.authority(Some(revoked_session));
        let negatives = [
            (
                "createSession",
                status_of(conn.block_on(
                    as_revoked(None).create_session(revoked.id(), &IdempotencyKey::new()),
                )),
            ),
            (
                "renewSession",
                status_of(
                    conn.block_on(
                        as_revoked(Some(revoked_session))
                            .renew_session(revoked_session.session_id.0, &IdempotencyKey::new()),
                    ),
                ),
            ),
            (
                "rebindSession",
                api.rebind_session_raw(&revoked_with_session, auth.workspace, revoked_session)
                    .status,
            ),
            (
                "acquireWorkflowLease",
                api.acquire_lease_raw(&revoked_with_session, auth.workspace)
                    .status,
            ),
        ];
        for (operation, status) in negatives {
            assert!(
                matches!(status, 401 | 403),
                "{operation} with the revoked bearer returned {status}, expected 401/403"
            );
        }
        self.evidence.phase(
            "5-case5-6",
            "revoked_bearer_statuses",
            json!(
                negatives
                    .iter()
                    .map(|(o, s)| json!({"operation": o, "status": s}))
                    .collect::<Vec<_>>()
            ),
        );
        conn.stat(client, "proof/successor.txt");
        conn.exec_ok(
            &pod_d,
            &format!("echo more | dd of={WORKSPACE_PATH}/proof/successor.txt oflag=append conv=notrunc,fsync status=none"),
        );
        let successor = conn.read_file(client, "proof/successor.txt");
        assert_eq!(successor, b"successor\nmore\n", "worker-2 keeps working");

        // (e) the lease is released only after pod D (bound to it) has finished phase 6:
        // a bound session fails closed after release/takeover (Stage 2a), so releasing
        // here would fence pod D's later writes. Pod C's loop is killed by delete_pod's
        // quiesce step.
        self.delete_pod(conn, &pod_c);

        // (f) no byte was published after revocation.
        let (final_size, final_generation) = conn.stat(client, fence_log);
        assert_eq!(
            (final_size, final_generation),
            (size_at_revocation, generation_at_revocation),
            "fence.log changed after the revocation commit"
        );
        self.evidence.phase(
            "5-case5-6",
            "final",
            json!({"size": final_size, "generation": final_generation}),
        );
        (pod_d, lease)
    }

    pub fn case7_ephemeral_root(
        &mut self,
        conn: &Connected,
        auth: &Principals,
        client: &Client,
        pod_d: WorkerPod,
        lease: WorkflowAuthority,
    ) {
        self.enter("6-case7-ephemeral-root");
        // Nothing else renews the 600 s lease that pod D's bound writes depend on, and phase 5
        // may have spent several minutes in pod creation; renew before relying on it.
        let api = &conn.api;
        let host = auth.host_authority();
        let renewed_epoch = api.renew_lease(&host, auth.workspace, &lease);
        assert_eq!(
            renewed_epoch, lease.epoch,
            "lease renewal must not change the mutation epoch"
        );
        self.evidence
            .phase("6-case7", "lease_renewed", json!({"epoch": renewed_epoch}));
        conn.exec_ok(&pod_d, "echo ephemeral > /tmp/outside.txt");
        conn.exec_ok(
            &pod_d,
            &format!("echo durable | dd of={WORKSPACE_PATH}/proof/inside.txt conv=fsync status=none; sync"),
        );
        self.delete_pod(conn, &pod_d);
        // Pod D (bound session) is gone; release the lease so the ordinary pod E can mutate.
        api.release_lease(&host, auth.workspace, &lease);
        self.evidence
            .phase("6-case7", "lease_released", json!({"epoch": lease.epoch}));
        let fresh = api.create_session(
            &auth.host.authority(None),
            auth.workspace,
            auth.workers[0].id(),
        );
        let pod_e = self.create_worker(
            conn,
            auth,
            Slot {
                tag: "e",
                node_index: 0,
                worker_index: 0,
            },
            &fresh,
            None,
        );
        let inside = conn.exec_ok(&pod_e, &format!("cat {WORKSPACE_PATH}/proof/inside.txt"));
        assert_eq!(inside.trim(), "durable", "inside.txt content on pod E");
        let outside = conn.pods.exec(&pod_e, "test ! -e /tmp/outside.txt");
        assert!(
            outside.success(),
            "/tmp/outside.txt must be absent on pod E"
        );
        let _ = conn.read_file(client, "proof/inside.txt");
        self.evidence.phase(
            "6-case7",
            "result",
            json!({"inside_txt": "durable", "outside_txt_absent": true}),
        );
        self.delete_pod(conn, &pod_e);
    }
}
