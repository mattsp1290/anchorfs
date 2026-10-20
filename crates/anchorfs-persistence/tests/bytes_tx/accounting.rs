use super::support::*;
use anchorfs_core::{ObjectId, ReservationId, sha256};
use anchorfs_persistence::{
    objects, quota,
    reservations::{self, QuotaKind, ReservationError},
};
use std::time::Duration;

#[tokio::test]
async fn reserve_charges_workspace_and_deployment_and_bounds_principal() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    assert!(matches!(
        reservations::insert(&mut tx, w, operation, &actor, 4097, Duration::from_secs(60)).await,
        Err(ReservationError::PrincipalBound)
    ));
    let r = reservations::insert(&mut tx, w, operation, &actor, 800, Duration::from_secs(60))
        .await
        .unwrap();
    reservations::charge(&mut tx, w, &r).await.unwrap();
    let values: (i64, i64) = sqlx::query_as("SELECT w.staging_bytes_reserved,d.staging_bytes_reserved FROM workspace_quotas w CROSS JOIN deployment_quotas d WHERE w.workspace_id=$1")
        .bind(w.0).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(values, (800, 800));
    sqlx::query("UPDATE workspace_quotas SET physical_bytes_limit=1000 WHERE workspace_id=$1")
        .bind(w.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(matches!(
        quota::reserve_staging(&mut tx, w, 201).await,
        Err(ReservationError::Exceeded(QuotaKind::WorkspaceStaging))
    ));
    sqlx::query("UPDATE workspace_quotas SET staging_bytes_limit=5000,physical_bytes_limit=2000 WHERE workspace_id=$1")
        .bind(w.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(matches!(
        quota::reserve_staging(&mut tx, w, 1201).await,
        Err(ReservationError::Exceeded(QuotaKind::WorkspacePhysical))
    ));
    tx.rollback().await.unwrap();
    for (staging, physical, expected) in [
        (100, 2000, QuotaKind::DeploymentStaging),
        (100, 100, QuotaKind::DeploymentStaging),
        (1000, 100, QuotaKind::DeploymentPhysical),
    ] {
        let mut tx = db.pool.begin().await.unwrap();
        sqlx::query("UPDATE deployment_quotas SET staging_bytes_limit=$1,physical_bytes_limit=$2")
            .bind(staging as i64)
            .bind(physical as i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        assert!(
            matches!(quota::reserve_staging(&mut tx, w, 200).await, Err(ReservationError::Exceeded(kind)) if kind==expected)
        );
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn orphan_charges_everything_not_skipped_and_counts_new_rows_once() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let r = reservations::insert(&mut tx, w, operation, &actor, 12, Duration::from_secs(60))
        .await
        .unwrap();
    let ids = [ObjectId::new(), ObjectId::new(), ObjectId::new()];
    for id in ids {
        objects::stage(
            &mut tx,
            w,
            ReservationId(r.id),
            operation,
            id,
            4,
            sha256(b"data"),
        )
        .await
        .unwrap();
    }
    reservations::charge(&mut tx, w, &r).await.unwrap();
    objects::mark_uploaded(&mut tx, w, &ids[..1]).await.unwrap();
    reservations::fail(&mut tx, w, &r, "failed").await.unwrap();
    let bytes = objects::orphan(
        &mut tx,
        w,
        ReservationId(r.id),
        &ids[1..2],
        &ids[2..],
        "synthetic-failure",
    )
    .await
    .unwrap();
    assert_eq!(bytes, 8);
    assert_eq!(
        objects::orphan(
            &mut tx,
            w,
            ReservationId(r.id),
            &ids[1..2],
            &ids[2..],
            "synthetic-failure"
        )
        .await
        .unwrap(),
        0
    );
    quota::release_to_orphans(&mut tx, w, 12, bytes)
        .await
        .unwrap();
    let values: (i64, i64, i64, i64) = sqlx::query_as("SELECT w.staging_bytes_reserved,w.physical_bytes_used,d.staging_bytes_reserved,d.physical_bytes_used FROM workspace_quotas w CROSS JOIN deployment_quotas d WHERE w.workspace_id=$1")
        .bind(w.0).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(values, (0, 8, 0, 8));
    let states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM objects WHERE workspace_id=$1 AND id=$2")
            .bind(w.0)
            .bind(ids[1].0)
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(states, ["uncertain"]);
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn stage_requires_active_reservation_and_finalize_ignores_expiry() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let r = reservations::insert(&mut tx, w, operation, &actor, 4, Duration::from_secs(60))
        .await
        .unwrap();
    sqlx::query("UPDATE upload_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1").bind(w.0).execute(&mut *tx).await.unwrap();
    assert!(matches!(
        reservations::lock_active(&mut tx, w, ReservationId(r.id), operation).await,
        Err(ReservationError::Expired)
    ));
    assert!(matches!(
        objects::stage(
            &mut tx,
            w,
            ReservationId(r.id),
            operation,
            ObjectId::new(),
            4,
            sha256(b"data")
        )
        .await,
        Err(ReservationError::Expired)
    ));
    let r = reservations::lock_for_finalize(&mut tx, w, ReservationId(r.id), operation)
        .await
        .unwrap();
    reservations::fail(&mut tx, w, &r, "expired").await.unwrap();
    assert!(matches!(
        objects::stage(
            &mut tx,
            w,
            ReservationId(r.id),
            operation,
            ObjectId::new(),
            4,
            sha256(b"data")
        )
        .await,
        Err(ReservationError::NotActive)
    ));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn settle_transfers_staging_to_physical_and_logical() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let r = reservations::insert(&mut tx, w, operation, &actor, 4, Duration::from_secs(60))
        .await
        .unwrap();
    let object = ObjectId::new();
    objects::stage(
        &mut tx,
        w,
        ReservationId(r.id),
        operation,
        object,
        4,
        sha256(b"data"),
    )
    .await
    .unwrap();
    reservations::charge(&mut tx, w, &r).await.unwrap();
    objects::mark_uploaded(&mut tx, w, &[object]).await.unwrap();
    reservations::consume(&mut tx, w, &r).await.unwrap();
    assert_eq!(
        objects::publish(&mut tx, w, ReservationId(r.id))
            .await
            .unwrap(),
        4
    );
    quota::settle(&mut tx, w, 4, 4, 4).await.unwrap();
    let values: (i64, i64, i64, i64, i64) = sqlx::query_as("SELECT w.staging_bytes_reserved,w.physical_bytes_used,w.logical_bytes_used,d.staging_bytes_reserved,d.physical_bytes_used FROM workspace_quotas w CROSS JOIN deployment_quotas d WHERE w.workspace_id=$1")
        .bind(w.0).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(values, (0, 4, 4, 0, 4));
    tx.rollback().await.unwrap();
}
