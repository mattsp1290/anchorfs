//! Regression for workspace FK / reservation / receipt lock-order interaction.
use super::support::*;
use anchorfs_core::{ObjectId, ReservationId, sha256};
use anchorfs_persistence::{
    authority, invariants, objects,
    receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope},
    reservations, workflows,
};
use std::time::Duration;

async fn wait_blocked(pool: &sqlx::PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND state='active' AND wait_event_type='Lock')").bind(pid).fetch_one(pool).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("transaction reached controlled lock wait");
}
fn scope(session: &authority::ActorSession) -> ReceiptScope {
    ReceiptScope::Workspace {
        workspace: session.workspace_id,
        credential: session.credential_id,
        session: session.id,
        session_epoch: session.session_epoch,
        authorization_epoch: 1,
        mutation_epoch: 1,
    }
}
#[tokio::test]
async fn object_staging_receipt_fence_and_lease_have_no_fk_lock_cycle() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let actor = actor(&mut setup, &session).await;
    let op = operation(&mut setup, &session).await;
    let r = reservations::insert(&mut setup, w, op, &actor, 4, Duration::from_secs(60))
        .await
        .unwrap();
    reservations::charge(&mut setup, w, &r).await.unwrap();
    setup.commit().await.unwrap();
    let key: String = sqlx::query_scalar(
        "SELECT idempotency_key FROM operations WHERE workspace_id=$1 AND id=$2",
    )
    .bind(w.0)
    .bind(op.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let rid = ReservationId(r.id);
    let mut lease = db.pool.begin().await.unwrap();
    authority::lock_workspace_for_update(&mut lease, w)
        .await
        .unwrap()
        .unwrap();
    let lease_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lease)
        .await
        .unwrap();
    let mut stage = db.pool.begin().await.unwrap();
    let stage_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *stage)
        .await
        .unwrap();
    sqlx::query("SET LOCAL lock_timeout='3s'")
        .execute(&mut *stage)
        .await
        .unwrap();
    let stager = tokio::spawn(async move {
        let object = objects::stage(&mut stage, w, rid, op, ObjectId::new(), 4, sha256(b"data"))
            .await
            .unwrap();
        stage.commit().await.unwrap();
        object
    });
    // Staging must wait on the workspace before taking reservation SHARE.
    // Without the fix it reaches the same wait via INSERT's implicit FK lock,
    // but retains reservation SHARE and blocks the fence below.
    wait_blocked(&db.pool, stage_pid).await;
    let mut fence = db.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='1s'")
        .execute(&mut *fence)
        .await
        .unwrap();
    assert!(matches!(
        receipts::fence_expired_pending(&mut fence, w, op)
            .await
            .unwrap(),
        receipts::FenceOutcome::StillActive
    ));
    let actor_session = session.clone();
    let lease_attempt = tokio::spawn(async move {
        let result = receipts::begin(
            &mut lease,
            scope(&actor_session),
            ReceiptRequest {
                key: &key,
                op_type: "workflow_acquire",
                digest: [2; 32],
                credential_epoch: 1,
                retention: Duration::from_secs(3600),
                wait_bound: Duration::from_secs(2),
            },
        )
        .await
        .unwrap();
        assert!(matches!(result, ReceiptEntry::Mismatch { .. }));
        lease.rollback().await.unwrap();
    });
    wait_blocked(&db.pool, lease_pid).await;
    fence.commit().await.unwrap();
    lease_attempt.await.unwrap();
    let staged = stager.await.unwrap();
    assert_eq!(staged.state, "staged");
    // Expiry cleanup remains authoritative after staging completes.
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(rid.0).execute(&db.pool).await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::fence_expired_pending(&mut tx, w, op)
            .await
            .unwrap(),
        receipts::FenceOutcome::Fenced
    ));
    tx.commit().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let lock = authority::lock_workspace_for_update(&mut tx, w)
        .await
        .unwrap()
        .unwrap();
    let grant = workflows::acquire(&mut tx, &lock, &actor, &session, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(grant.mutation_epoch, 2);
    tx.commit().await.unwrap();
    let observed:(String,i64,i64,i64)=sqlx::query_as("SELECT o.state,q.staging_bytes_reserved,q.physical_bytes_used,(SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1) FROM operations o JOIN workspace_quotas q USING(workspace_id) WHERE o.workspace_id=$1 AND o.id=$2").bind(w.0).bind(op.0).fetch_one(&db.pool).await.unwrap();
    assert_eq!(observed, ("failed".into(), 0, 4, 1));
    assert!(invariants::check(&db.pool, w).await.unwrap().is_empty());
    assert!(
        invariants::check_deployment(&db.pool)
            .await
            .unwrap()
            .is_empty()
    );
}
