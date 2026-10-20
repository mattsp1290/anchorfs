use super::support::*;
use anchorfs_core::{
    ChunkGeometry, ManifestId, ObjectId, ReadPinId, ReservationId, WorkspacePath, plan_truncate,
    sha256,
};
use anchorfs_persistence::{
    manifests::{self, ManifestError, NewChunk},
    namespace, objects,
    pins::{self, PinError},
    reservations,
};
use std::time::Duration;

#[tokio::test]
async fn publish_write_copies_untouched_rows_and_bumps_generation() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let inode = namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .create_file(&caller(), &WorkspacePath::parse("f").unwrap(), 0o600)
        .await
        .unwrap();
    let (empty, inode) = manifests::publish_empty(&mut tx, w, &inode, operation, 4096, false)
        .await
        .unwrap();
    assert_eq!(empty.size, 0);
    assert_eq!(empty.generation, 1);
    let r = reservations::insert(&mut tx, w, operation, &actor, 12, Duration::from_secs(60))
        .await
        .unwrap();
    let mut chunks = Vec::new();
    for index in 0..3 {
        let id = ObjectId::new();
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
        chunks.push(NewChunk {
            index,
            object: id,
            sha256: sha256(b"data"),
            logical_length: 4,
        });
    }
    let (old, inode) = manifests::publish_write(&mut tx, w, &inode, operation, 4096, 8196, &chunks)
        .await
        .unwrap();
    let id = ObjectId::new();
    objects::stage(
        &mut tx,
        w,
        ReservationId(r.id),
        operation,
        id,
        4,
        sha256(b"next"),
    )
    .await
    .unwrap();
    let (new, inode) = manifests::publish_write(
        &mut tx,
        w,
        &inode,
        operation,
        4096,
        8196,
        &[NewChunk {
            index: 1,
            object: id,
            sha256: sha256(b"next"),
            logical_length: 4,
        }],
    )
    .await
    .unwrap();
    assert_eq!(new.generation, inode.generation);
    assert_eq!(new.generation, 3);
    let rows = manifests::chunks(&mut tx, w, ManifestId(new.id), None)
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.object_id).collect::<Vec<_>>(),
        [chunks[0].object.0, id.0, chunks[2].object.0]
    );
    let rows = manifests::chunks(&mut tx, w, ManifestId(old.id), None)
        .await
        .unwrap();
    assert_eq!(rows[1].object_id, chunks[1].object.0);
    assert!(
        manifests::chunks(&mut tx, w, ManifestId(empty.id), None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        manifests::publish_write(&mut tx, w, &inode, operation, 8192, 8196, &[]).await,
        Err(ManifestError::ChunkSizeMismatch)
    ));
    let (replaced, inode) = manifests::publish_replace(
        &mut tx,
        w,
        &inode,
        operation,
        4096,
        4,
        &[NewChunk {
            index: 0,
            object: id,
            sha256: sha256(b"next"),
            logical_length: 4,
        }],
    )
    .await
    .unwrap();
    assert_eq!(replaced.generation, 4);
    assert_eq!(inode.size, 4);
    let rows = manifests::chunks(&mut tx, w, ManifestId(replaced.id), Some(&[0, 2]))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].object_id, id.0);
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn publish_truncate_clips_boundary_and_drops_tail_with_high_index() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let inode = namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .create_file(&caller(), &WorkspacePath::parse("f").unwrap(), 0o600)
        .await
        .unwrap();
    let (_, inode) = manifests::publish_empty(&mut tx, w, &inode, operation, 4194304, false)
        .await
        .unwrap();
    let r = reservations::insert(&mut tx, w, operation, &actor, 16, Duration::from_secs(60))
        .await
        .unwrap();
    let mut rows = Vec::new();
    for index in [600, 601] {
        let id = ObjectId::new();
        objects::stage(
            &mut tx,
            w,
            ReservationId(r.id),
            operation,
            id,
            8,
            sha256(b"abcdefgh"),
        )
        .await
        .unwrap();
        rows.push(NewChunk {
            index,
            object: id,
            sha256: sha256(b"abcdefgh"),
            logical_length: 8,
        });
    }
    let size = 601u64 * 4194304 + 8;
    let (_, inode) = manifests::publish_write(&mut tx, w, &inode, operation, 4194304, size, &rows)
        .await
        .unwrap();
    let geometry = ChunkGeometry {
        chunk_size: 4194304,
        max_file_size: 1u64 << 40,
    };
    let existing = [
        anchorfs_core::ExistingChunk {
            index: 600,
            logical_length: 8,
            object_length: 8,
        },
        anchorfs_core::ExistingChunk {
            index: 601,
            logical_length: 8,
            object_length: 8,
        },
    ];
    let shrink = 600u64 * 4194304 + 3;
    let plan = plan_truncate(geometry, size, shrink, &existing).unwrap();
    let (manifest, inode) =
        manifests::publish_truncate(&mut tx, w, &inode, operation, shrink, &plan)
            .await
            .unwrap();
    let chunks = manifests::chunks(&mut tx, w, ManifestId(manifest.id), None)
        .await
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].object_id, rows[0].object.0);
    assert_eq!(chunks[0].logical_length, 3);
    assert_eq!(chunks[0].object_length, 8);
    let plan = plan_truncate(geometry, shrink, size, &plan.kept).unwrap();
    let (grown, inode) = manifests::publish_truncate(&mut tx, w, &inode, operation, size, &plan)
        .await
        .unwrap();
    let chunks = manifests::chunks(&mut tx, w, ManifestId(grown.id), None)
        .await
        .unwrap();
    assert_eq!(chunks[0].logical_length, 3);
    let plan = plan_truncate(geometry, size, size, &plan.kept).unwrap();
    let (same, inode) = manifests::publish_truncate(&mut tx, w, &inode, operation, size, &plan)
        .await
        .unwrap();
    assert_eq!(same.id, grown.id);
    let (empty, inode) = manifests::publish_empty(&mut tx, w, &inode, operation, 4194304, true)
        .await
        .unwrap();
    assert_eq!(empty.size, 0);
    assert_eq!(inode.generation, grown.generation + 1);
    assert!(
        manifests::chunks(&mut tx, w, ManifestId(empty.id), None)
            .await
            .unwrap()
            .is_empty()
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn pins_bound_per_session_and_detect_mismatch() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = actor(&mut tx, &session).await;
    let grant = anchorfs_persistence::sessions::create(
        &mut tx,
        &actor,
        w,
        actor.id,
        Duration::from_secs(60),
        64,
    )
    .await
    .unwrap();
    let other = anchorfs_persistence::authority::load_actor_session(
        &mut tx,
        &actor,
        w,
        grant.id,
        1,
        anchorfs_persistence::authority::LockMode::Share,
    )
    .await
    .unwrap();
    let operation = operation(&mut tx, &session).await;
    let inode = namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .create_file(&caller(), &WorkspacePath::parse("f").unwrap(), 0o600)
        .await
        .unwrap();
    let (manifest, inode) = manifests::publish_empty(&mut tx, w, &inode, operation, 4096, false)
        .await
        .unwrap();
    let pin = pins::create(
        &mut tx,
        w,
        &session,
        ManifestId(manifest.id),
        Duration::from_secs(60),
        2,
    )
    .await
    .unwrap();
    pins::create(
        &mut tx,
        w,
        &session,
        ManifestId(manifest.id),
        Duration::from_secs(60),
        2,
    )
    .await
    .unwrap();
    assert!(matches!(
        pins::create(
            &mut tx,
            w,
            &session,
            ManifestId(manifest.id),
            Duration::from_secs(60),
            2
        )
        .await,
        Err(PinError::Limit)
    ));
    assert!(matches!(
        pins::load_for_read(&mut tx, w, &session, ReadPinId(pin.id), inode.id + 1).await,
        Err(PinError::Mismatch)
    ));
    assert!(matches!(
        pins::load_for_read(&mut tx, w, &other, ReadPinId(pin.id), inode.id).await,
        Err(PinError::NotFound)
    ));
    assert_eq!(
        pins::load_for_read(&mut tx, w, &session, ReadPinId(pin.id), inode.id)
            .await
            .unwrap()
            .1
            .id,
        manifest.id
    );
    pins::release(&mut tx, w, &session, ReadPinId(pin.id))
        .await
        .unwrap();
    assert!(matches!(
        pins::load_for_read(&mut tx, w, &session, ReadPinId(pin.id), inode.id).await,
        Err(PinError::Stale)
    ));
    assert_eq!(pins::count_live(&mut tx, w, session.id).await.unwrap(), 1);
    tx.rollback().await.unwrap();
}
