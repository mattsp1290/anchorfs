use super::support::*;
use anchorfs_core::{ManifestId, ObjectId, ReservationId, WorkspacePath, sha256};
use anchorfs_persistence::{
    invariants,
    manifests::{self, NewChunk},
    namespace, objects, quota, reservations,
};
use std::time::Duration;

#[tokio::test]
async fn invariants_detect_each_byte_violation() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let inode = namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .create_file(
            &caller(),
            &WorkspacePath::parse("f").unwrap(),
            0o600,
            operation,
            4096,
        )
        .await
        .unwrap();
    let reservation =
        reservations::insert(&mut tx, w, operation, &actor, 4, Duration::from_secs(60))
            .await
            .unwrap();
    let object = ObjectId::new();
    objects::stage(
        &mut tx,
        w,
        ReservationId(reservation.id),
        operation,
        object,
        4,
        sha256(b"data"),
    )
    .await
    .unwrap();
    reservations::charge(&mut tx, w, &reservation)
        .await
        .unwrap();
    objects::mark_uploaded(&mut tx, w, &[object]).await.unwrap();
    objects::publish(&mut tx, w, ReservationId(reservation.id))
        .await
        .unwrap();
    reservations::consume(&mut tx, w, &reservation)
        .await
        .unwrap();
    let (manifest, _) = manifests::publish_write(
        &mut tx,
        w,
        &inode,
        operation,
        4096,
        4,
        &[NewChunk {
            index: 0,
            object,
            sha256: sha256(b"data"),
            logical_length: 4,
        }],
    )
    .await
    .unwrap();
    quota::settle(&mut tx, w, 4, 4, 4).await.unwrap();
    tx.commit().await.unwrap();
    assert!(invariants::check(&db.pool, w).await.unwrap().is_empty());
    assert!(
        invariants::check_deployment(&db.pool)
            .await
            .unwrap()
            .is_empty()
    );
    for (sql, expected) in [
        (
            "UPDATE inodes SET current_manifest_id=NULL WHERE workspace_id=$1 AND kind='file'",
            format!("file-manifest:{}", inode.id),
        ),
        (
            "UPDATE inodes SET size=5 WHERE workspace_id=$1 AND kind='file'",
            format!("manifest-size:{}", inode.id),
        ),
        (
            "UPDATE inodes SET generation=1 WHERE workspace_id=$1 AND kind='file'",
            format!("manifest-generation:{}", inode.id),
        ),
        (
            "INSERT INTO manifest_chunks SELECT workspace_id,manifest_id,600,chunk_size,object_id,sha256,logical_length FROM manifest_chunks WHERE workspace_id=$1",
            format!("chunk-extent:{}", manifest.id),
        ),
        (
            "UPDATE objects SET byte_length=3 WHERE workspace_id=$1",
            format!("chunk-extent:{}", manifest.id),
        ),
        (
            "UPDATE objects SET state='staged' WHERE workspace_id=$1",
            format!("chunk-object:{}", manifest.id),
        ),
        (
            "INSERT INTO deletion_candidates SELECT workspace_id,id,'synthetic-negative-control',byte_length,clock_timestamp() FROM objects WHERE workspace_id=$1",
            format!("candidate-published:{}", object.0),
        ),
        (
            "UPDATE workspace_quotas SET staging_bytes_reserved=1 WHERE workspace_id=$1",
            "staging".into(),
        ),
        (
            "UPDATE workspace_quotas SET physical_bytes_used=5 WHERE workspace_id=$1",
            "physical".into(),
        ),
    ] {
        let mut tx = db.pool.begin().await.unwrap();
        sqlx::query(sql).bind(w.0).execute(&mut *tx).await.unwrap();
        let violations = invariants::check(&mut *tx, w).await.unwrap();
        assert!(
            violations.contains(&expected),
            "expected {expected}, got {violations:?}"
        );
        tx.rollback().await.unwrap();
    }
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE deployment_quotas SET staging_bytes_reserved=1,physical_bytes_used=5")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        invariants::check_deployment(&mut *tx).await.unwrap(),
        ["deployment-physical", "deployment-staging"]
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        manifests::chunks(
            &mut db.pool.begin().await.unwrap(),
            w,
            ManifestId(manifest.id),
            None
        )
        .await
        .unwrap()
        .len(),
        1
    );
    assert!(invariants::check(&db.pool, w).await.unwrap().is_empty());
}
