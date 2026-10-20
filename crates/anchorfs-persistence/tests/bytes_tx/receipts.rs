use super::support::*;
use anchorfs_core::{ObjectId, OperationId, ReservationId, Secret, WorkspaceId, sha256};
use anchorfs_persistence::{
    authority::{self, ActorSession, LockMode},
    objects, quota,
    receipts::{
        self, FenceOutcome, ReceiptEntry, ReceiptError, ReceiptFilter, ReceiptRequest, ReceiptScope,
    },
    reservations, sessions,
};
use std::time::Duration;

fn scope(session: &ActorSession) -> ReceiptScope {
    ReceiptScope::Workspace {
        workspace: session.workspace_id,
        credential: session.credential_id,
        session: session.id,
        session_epoch: session.session_epoch,
        authorization_epoch: 1,
        mutation_epoch: 1,
    }
}
fn request() -> ReceiptRequest<'static> {
    ReceiptRequest {
        key: "byte-key",
        op_type: "write",
        digest: [17; 32],
        credential_epoch: 1,
        retention: Duration::from_secs(3600),
        wait_bound: Duration::from_secs(1),
    }
}
fn filter(session: &ActorSession) -> ReceiptFilter {
    ReceiptFilter {
        workspace: Some(session.workspace_id),
        credential: session.credential_id,
        allow_workspace_actors: false,
    }
}
struct Pending {
    db: crate::common::Database,
    workspace: WorkspaceId,
    session: ActorSession,
    operation: OperationId,
    owner: Secret<String>,
    reservation: reservations::ReservationRow,
    object: ObjectId,
}
async fn pending() -> Pending {
    let (db, workspace) = fixture().await;
    let session = actor_session(&db, workspace).await;
    let mut tx = db.pool.begin().await.unwrap();
    let entry = receipts::begin(&mut tx, scope(&session), request())
        .await
        .unwrap();
    let debug = format!("{entry:?}");
    let ReceiptEntry::Fresh {
        operation_id: operation,
        owner,
    } = entry
    else {
        panic!("fresh operation expected");
    };
    assert!(
        !debug.contains(owner.expose()),
        "receipt Debug must redact ownership proof"
    );
    let actor = actor(&mut tx, &session).await;
    let reservation = reservations::insert(
        &mut tx,
        workspace,
        operation,
        &actor,
        4,
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let object = ObjectId::new();
    objects::stage(
        &mut tx,
        workspace,
        ReservationId(reservation.id),
        operation,
        object,
        4,
        sha256(b"data"),
    )
    .await
    .unwrap();
    reservations::charge(&mut tx, workspace, &reservation)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Pending {
        db,
        workspace,
        session,
        operation,
        owner,
        reservation,
        object,
    }
}
async fn expire_reservation(p: &Pending) {
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=$2")
        .bind(p.workspace.0).bind(p.reservation.id).execute(&p.db.pool).await.unwrap();
}
async fn counters(pool: &sqlx::PgPool, workspace: WorkspaceId) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT w.staging_bytes_reserved,w.physical_bytes_used,d.staging_bytes_reserved,d.physical_bytes_used FROM workspace_quotas w CROSS JOIN deployment_quotas d WHERE w.workspace_id=$1")
        .bind(workspace.0).fetch_one(pool).await.unwrap()
}
#[tokio::test]
async fn receipt_fresh_returns_owner_and_lock_pending_requires_it() {
    let p = pending().await;
    assert!(!format!("{:?}", p.owner).contains(p.owner.expose()));
    let mut tx = p.db.pool.begin().await.unwrap();
    let op = receipts::lock_pending(&mut tx, scope(&p.session), p.operation, &p.owner)
        .await
        .unwrap();
    assert_eq!(op.operation_id, p.operation.0);
    assert_eq!(op.mutation_epoch_at_start, 1);
    assert_eq!(op.authorization_epoch_at_start, 1);
    assert!(matches!(
        receipts::lock_pending(
            &mut tx,
            scope(&p.session),
            p.operation,
            &Secret::new("wrong".into())
        )
        .await,
        Err(ReceiptError::NotOwner)
    ));
    assert!(
        receipts::fail_result(&mut tx, scope(&p.session), p.operation, "invalid_input")
            .await
            .unwrap()
    );
    assert!(
        matches!(receipts::begin(&mut tx, scope(&p.session), request()).await.unwrap(), ReceiptEntry::Failed { operation_id } if operation_id==p.operation)
    );
    assert!(matches!(
        receipts::peek(&mut tx, scope(&p.session), "byte-key", [17; 32])
            .await
            .unwrap(),
        Some(ReceiptEntry::Failed { .. })
    ));
    assert!(matches!(
        receipts::lock_pending(&mut tx, scope(&p.session), p.operation, &p.owner).await,
        Err(ReceiptError::NotOwner)
    ));
    tx.rollback().await.unwrap();
    let mut tx = p.db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::peek(&mut tx, scope(&p.session), "byte-key", [17; 32])
            .await
            .unwrap(),
        Some(ReceiptEntry::Pending {
            reservation_overdue: false,
            ..
        })
    ));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn pending_receipt_with_expired_reservation_is_fenced_once() {
    let p = pending().await;
    let mut tx = p.db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::begin(&mut tx, scope(&p.session), request())
            .await
            .unwrap(),
        ReceiptEntry::Pending {
            reservation_overdue: false,
            ..
        }
    ));
    tx.commit().await.unwrap();
    expire_reservation(&p).await;
    let mut tx = p.db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::peek(&mut tx, scope(&p.session), "byte-key", [17; 32])
            .await
            .unwrap(),
        Some(ReceiptEntry::Pending {
            reservation_overdue: true,
            ..
        })
    ));
    tx.rollback().await.unwrap();
    assert_eq!(counters(&p.db.pool, p.workspace).await, (4, 0, 4, 0));
    let mut tx = p.db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::begin(&mut tx, scope(&p.session), request())
            .await
            .unwrap(),
        ReceiptEntry::Failed { .. }
    ));
    tx.commit().await.unwrap();
    let candidate: (String,i64) = sqlx::query_as("SELECT r.state,c.physical_bytes FROM upload_reservations r JOIN objects o ON (o.workspace_id,o.reservation_id)=(r.workspace_id,r.id) JOIN deletion_candidates c ON (c.workspace_id,c.object_id)=(o.workspace_id,o.id) WHERE r.workspace_id=$1 AND o.id=$2")
        .bind(p.workspace.0).bind(p.object.0).fetch_one(&p.db.pool).await.unwrap();
    assert_eq!(candidate, ("expired".into(), 4));
    assert_eq!(counters(&p.db.pool, p.workspace).await, (0, 4, 0, 4));
    let mut tx = p.db.pool.begin().await.unwrap();
    assert!(matches!(
        receipts::begin(&mut tx, scope(&p.session), request())
            .await
            .unwrap(),
        ReceiptEntry::Failed { .. }
    ));
    tx.commit().await.unwrap();
    assert_eq!(counters(&p.db.pool, p.workspace).await, (0, 4, 0, 4));
}

#[tokio::test]
async fn inspection_and_quota_pressure_fence_overdue_operations() {
    for inspect in [true, false] {
        let p = pending().await;
        expire_reservation(&p).await;
        if inspect {
            let mut tx = p.db.pool.begin().await.unwrap();
            let view = receipts::read_by_id(&mut tx, filter(&p.session), p.operation)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(view.state, "failed");
            tx.commit().await.unwrap();
        } else {
            assert_eq!(
                reservations::fence_overdue(&p.db.pool, Some(p.workspace), 10)
                    .await
                    .unwrap(),
                1
            );
            assert_eq!(
                reservations::fence_overdue(&p.db.pool, Some(p.workspace), 10)
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(counters(&p.db.pool, p.workspace).await, (0, 4, 0, 4));
        let mut tx = p.db.pool.begin().await.unwrap();
        quota::reserve_staging(&mut tx, p.workspace, 4)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn conditional_receipt_writers_change_zero_rows_when_state_moved() {
    let p = pending().await;
    let mut tx = p.db.pool.begin().await.unwrap();
    let result = serde_json::json!({"offset":7});
    assert!(
        receipts::commit_result(&mut tx, scope(&p.session), p.operation, &result)
            .await
            .unwrap()
    );
    assert!(
        !receipts::fail_result(&mut tx, scope(&p.session), p.operation, "invalid_input")
            .await
            .unwrap()
    );
    assert!(
        !receipts::commit_result(
            &mut tx,
            scope(&p.session),
            p.operation,
            &serde_json::json!({"offset":99})
        )
        .await
        .unwrap()
    );
    let view = receipts::read_by_id(&mut tx, filter(&p.session), p.operation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.state, "committed");
    assert_eq!(view.result, Some(result));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn fence_does_not_overwrite_a_committed_receipt() {
    let p = pending().await;
    expire_reservation(&p).await;
    let mut publisher = p.db.pool.begin().await.unwrap();
    receipts::lock_pending(&mut publisher, scope(&p.session), p.operation, &p.owner)
        .await
        .unwrap();
    objects::mark_uploaded(&mut publisher, p.workspace, &[p.object])
        .await
        .unwrap();
    objects::publish(&mut publisher, p.workspace, ReservationId(p.reservation.id))
        .await
        .unwrap();
    reservations::consume(&mut publisher, p.workspace, &p.reservation)
        .await
        .unwrap();
    quota::settle(&mut publisher, p.workspace, 4, 4, 0)
        .await
        .unwrap();
    let result = serde_json::json!({"offset":10});
    assert!(
        receipts::commit_result(&mut publisher, scope(&p.session), p.operation, &result)
            .await
            .unwrap()
    );
    let mut waiter = p.db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *waiter)
        .await
        .unwrap();
    let w = p.workspace;
    let operation = p.operation;
    let waiting = tokio::spawn(async move {
        let outcome = receipts::fence_expired_pending(&mut waiter, w, operation)
            .await
            .unwrap();
        waiter.commit().await.unwrap();
        outcome
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')")
                .bind(pid).fetch_one(&p.db.pool).await.unwrap();
            if blocked { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    publisher.commit().await.unwrap();
    let outcome = waiting.await.unwrap();
    assert!(
        matches!(outcome, FenceOutcome::AlreadyResolved(row) if row.state=="committed" && row.result==Some(result))
    );
    assert_eq!(counters(&p.db.pool, p.workspace).await, (0, 4, 0, 4));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM deletion_candidates WHERE workspace_id=$1")
            .bind(p.workspace.0)
            .fetch_one(&p.db.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn expired_receipt_tombstone_then_session_fence_once() {
    for prior in ["committed", "failed", "pending"] {
        let p = pending().await;
        if prior == "pending" {
            expire_reservation(&p).await;
        }
        let mut tx = p.db.pool.begin().await.unwrap();
        if prior == "committed" {
            receipts::commit_result(
                &mut tx,
                scope(&p.session),
                p.operation,
                &serde_json::json!({"ok":true}),
            )
            .await
            .unwrap();
        } else if prior == "failed" {
            receipts::fail_result(&mut tx, scope(&p.session), p.operation, "invalid_input")
                .await
                .unwrap();
        }
        sqlx::query("UPDATE operations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=$2")
            .bind(p.workspace.0).bind(p.operation.0).execute(&mut *tx).await.unwrap();
        let view = receipts::read_by_id(&mut tx, filter(&p.session), p.operation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(view.state, "indeterminate");
        assert!(view.result.is_none());
        let tombstoned = view.tombstoned.unwrap();
        assert_eq!(tombstoned.prior_state, prior);
        assert_eq!(
            tombstoned.session,
            if prior == "failed" {
                None
            } else {
                Some(p.session.id)
            }
        );
        tx.commit().await.unwrap();
        // Simulate death before the follow-up: eligibility survives in the row.
        let mut tx = p.db.pool.begin().await.unwrap();
        let view = receipts::read_by_key(&mut tx, filter(&p.session), "byte-key")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            view.tombstoned.unwrap().session,
            if prior == "failed" {
                None
            } else {
                Some(p.session.id)
            }
        );
        tx.commit().await.unwrap();
        let mut tx = p.db.pool.begin().await.unwrap();
        assert_eq!(
            sessions::fence_for_expired_receipt(&mut tx, p.workspace, p.session.id, p.operation)
                .await
                .unwrap(),
            prior != "failed"
        );
        assert!(
            !sessions::fence_for_expired_receipt(&mut tx, p.workspace, p.session.id, p.operation)
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();
        let mut tx = p.db.pool.begin().await.unwrap();
        let actor = actor(&mut tx, &p.session).await;
        let loaded = authority::load_actor_session(
            &mut tx,
            &actor,
            p.workspace,
            p.session.id,
            1,
            LockMode::Share,
        )
        .await;
        if prior == "failed" {
            assert!(loaded.is_ok());
        } else {
            assert!(matches!(loaded, Err(authority::AuthorityError::Fenced)));
        }
        let epoch: i64 =
            sqlx::query_scalar("SELECT session_epoch FROM client_sessions WHERE id=$1")
                .bind(p.session.id.0)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(epoch, if prior == "failed" { 1 } else { 2 });
        tx.rollback().await.unwrap();
        if prior == "pending" {
            assert_eq!(counters(&p.db.pool, p.workspace).await, (0, 4, 0, 4));
        }
    }
}
