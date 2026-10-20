use super::support::*;
use anchorfs_core::{
    ChunkGeometry, HandleId, ManifestId, ObjectId, ReservationId, WorkspacePath, plan_truncate,
    sha256,
};
use anchorfs_persistence::{
    authority::LockMode,
    handles::{self, HandleError, HandlePolicy, OpenRequest},
    manifests::{self, NewChunk},
    namespace, objects, quota, reservations,
};
use std::time::Duration;

fn request(create: bool, truncate: bool) -> OpenRequest {
    OpenRequest {
        path: WorkspacePath::parse("f").unwrap(),
        read: true,
        write: true,
        create,
        exclusive: false,
        truncate,
        mode: Some(0o600),
    }
}
fn policy() -> HandlePolicy {
    HandlePolicy {
        ttl: Duration::from_secs(60),
        limit: 10,
    }
}
async fn waiting(pool: &sqlx::PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let blocked:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(pool).await.unwrap();
            if blocked { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn publish_empty_for_create_and_truncate_open() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let actor = actor(&mut tx, &session).await;
    let opened = handles::open_file(
        &mut tx,
        &caller(),
        &session,
        request(true, false),
        policy(),
        operation,
        4096,
    )
    .await
    .unwrap();
    let empty = manifests::current(&mut tx, w, &opened.inode).await.unwrap();
    assert_eq!((empty.size, empty.generation), (0, 1));
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
    let (old, inode) = manifests::publish_write(
        &mut tx,
        w,
        &opened.inode,
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
    let opened = handles::open_file(
        &mut tx,
        &caller(),
        &session,
        request(false, true),
        policy(),
        operation,
        4096,
    )
    .await
    .unwrap();
    assert_eq!(opened.inode.size, 0);
    assert_eq!(opened.inode.generation, inode.generation + 1);
    let current = manifests::current(&mut tx, w, &opened.inode).await.unwrap();
    assert!(
        manifests::chunks(&mut tx, w, ManifestId(current.id), None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        manifests::chunks(&mut tx, w, ManifestId(old.id), None)
            .await
            .unwrap()
            .len(),
        1
    );
    let counters: (i64, i64) = sqlx::query_as(
        "SELECT logical_bytes_used,physical_bytes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(w.0)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(counters, (0, 4));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM objects WHERE workspace_id=$1")
        .bind(w.0)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        anchorfs_persistence::invariants::check(&mut *tx, w)
            .await
            .unwrap()
            .is_empty()
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn handle_read_and_write_enforce_open_access() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let operation = operation(&mut tx, &session).await;
    let mut req = request(true, false);
    req.write = false;
    let read = handles::open_file(&mut tx, &caller(), &session, req, policy(), operation, 4096)
        .await
        .unwrap();
    assert!(
        handles::load_for_read(&mut tx, w, &session, HandleId(read.handle.id), None)
            .await
            .is_ok()
    );
    assert!(matches!(
        handles::load_for_write(&mut tx, w, &session, HandleId(read.handle.id), None).await,
        Err(HandleError::Forbidden)
    ));
    let mut req = request(false, false);
    req.read = false;
    let write = handles::open_file(&mut tx, &caller(), &session, req, policy(), operation, 4096)
        .await
        .unwrap();
    assert!(
        handles::load_for_write(
            &mut tx,
            w,
            &session,
            HandleId(write.handle.id),
            Some(LockMode::Share)
        )
        .await
        .is_ok()
    );
    assert!(matches!(
        handles::load_for_read(&mut tx, w, &session, HandleId(write.handle.id), None).await,
        Err(HandleError::Forbidden)
    ));
    handles::close(&mut tx, w, &session, HandleId(write.handle.id))
        .await
        .unwrap();
    assert!(matches!(
        handles::load_for_write(&mut tx, w, &session, HandleId(write.handle.id), None).await,
        Err(HandleError::Stale)
    ));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn close_waits_on_inode_held_by_publication() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let operation = operation(&mut setup, &session).await;
    let opened = handles::open_file(
        &mut setup,
        &caller(),
        &session,
        request(true, false),
        policy(),
        operation,
        4096,
    )
    .await
    .unwrap();
    setup.commit().await.unwrap();
    let mut publisher = db.pool.begin().await.unwrap();
    namespace::lock_inodes(&mut publisher, w, &[opened.inode.id])
        .await
        .unwrap();
    let mut closing = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *closing)
        .await
        .unwrap();
    let close = tokio::spawn(async move {
        let result = handles::close(&mut closing, w, &session, HandleId(opened.handle.id))
            .await
            .unwrap();
        closing.commit().await.unwrap();
        result
    });
    waiting(&db.pool, pid).await;
    publisher.commit().await.unwrap();
    let (_, inode) = close.await.unwrap();
    assert!(inode.nlink > 0);
    assert!(inode.charge_released_at.is_none());
    assert!(
        anchorfs_persistence::invariants::check(&db.pool, w)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn two_publishers_serialize_on_inode() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let operation = operation(&mut setup, &session).await;
    let opened = handles::open_file(
        &mut setup,
        &caller(),
        &session,
        request(true, false),
        policy(),
        operation,
        4096,
    )
    .await
    .unwrap();
    setup.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    let inode = namespace::lock_inodes(&mut first, w, &[opened.inode.id])
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let id = inode.id;
    let next = tokio::spawn(async move {
        let inode = namespace::lock_inodes(&mut second, w, &[id])
            .await
            .unwrap()
            .pop()
            .unwrap();
        second.rollback().await.unwrap();
        inode
    });
    waiting(&db.pool, pid).await;
    let plan = plan_truncate(
        ChunkGeometry {
            chunk_size: 4096,
            max_file_size: 1000,
        },
        0,
        4,
        &[],
    )
    .unwrap();
    let (_, published) = manifests::publish_truncate(&mut first, w, &inode, operation, 4, &plan)
        .await
        .unwrap();
    quota::apply(
        &mut first,
        w,
        quota::Charge {
            inodes: 0,
            bytes: 4,
        },
    )
    .await
    .unwrap();
    first.commit().await.unwrap();
    let observed = next.await.unwrap();
    assert_eq!(observed.generation, published.generation);
    assert_eq!(observed.current_manifest_id, published.current_manifest_id);
    assert_ne!(observed.current_manifest_id, inode.current_manifest_id);
    assert!(
        anchorfs_persistence::invariants::check(&db.pool, w)
            .await
            .unwrap()
            .is_empty()
    );
}
