use super::support::*;
use anchorfs_core::{HandleId, SessionId, WorkspacePath};
use anchorfs_persistence::{
    authority::ActorSession,
    handles::{self, HandleError, HandlePolicy, OpenRequest, Opened},
    namespace,
};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
fn path(s: &str) -> WorkspacePath {
    WorkspacePath::parse(s).unwrap()
}
fn policy() -> HandlePolicy {
    HandlePolicy {
        ttl: Duration::from_secs(60),
        limit: 10,
    }
}
async fn open(
    tx: &mut Transaction<'_, Postgres>,
    s: &ActorSession,
    name: &str,
    policy: HandlePolicy,
) -> Result<Opened, HandleError> {
    handles::open_file(
        tx,
        &caller(),
        s,
        OpenRequest {
            path: path(name),
            read: true,
            write: true,
            create: true,
            exclusive: false,
            truncate: false,
            mode: Some(0o600),
        },
        policy,
    )
    .await
}
#[tokio::test]
async fn close_releases_charge_only_after_last_open_handle() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let a = open(&mut tx, &s, "f", policy()).await.unwrap();
    let b = open(&mut tx, &s, "f", policy()).await.unwrap();

    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .unlink(&caller(), &path("f"))
        .await
        .unwrap();
    assert!(
        anchorfs_persistence::invariants::check(&mut *tx, w)
            .await
            .unwrap()
            .is_empty()
    );
    handles::close(&mut tx, w, &s, HandleId(a.handle.id))
        .await
        .unwrap();
    let released: bool = sqlx::query_scalar(
        "SELECT charge_released_at IS NOT NULL FROM inodes WHERE workspace_id=$1 AND id=$2",
    )
    .bind(w.0)
    .bind(a.inode.id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert!(!released);
    handles::renew(&mut tx, w, &s, HandleId(b.handle.id), policy().ttl)
        .await
        .unwrap();
    handles::close(&mut tx, w, &s, HandleId(b.handle.id))
        .await
        .unwrap();
    let used: i64 = sqlx::query_scalar(
        "SELECT logical_inodes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(w.0)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(used, 0);
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn rename_replaces_file_and_keeps_open_inode() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let a = open(&mut tx, &s, "a", policy()).await.unwrap();
    let b = open(&mut tx, &s, "b", policy()).await.unwrap();

    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .rename(&caller(), &path("a"), &path("b"))
        .await
        .unwrap();
    assert_eq!(
        namespace::resolve(&mut tx, w, Some(&path("b")), &caller())
            .await
            .unwrap()
            .id,
        a.inode.id
    );
    let (_, old) = handles::renew(&mut tx, w, &s, HandleId(b.handle.id), policy().ttl)
        .await
        .unwrap();
    assert_eq!(old.nlink, 0);
    assert!(old.charge_released_at.is_none());
    handles::close(&mut tx, w, &s, HandleId(b.handle.id))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn handle_invisible_to_other_session() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let a = open(&mut tx, &s, "f", policy()).await.unwrap();
    let other = ActorSession {
        id: SessionId::new(),
        ..s.clone()
    };
    assert!(matches!(
        handles::renew(&mut tx, w, &other, HandleId(a.handle.id), policy().ttl).await,
        Err(HandleError::NotFound)
    ));
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn renew_extends_and_stale_after_expiry() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut tx = db.pool.begin().await.unwrap();
    let a = open(
        &mut tx,
        &s,
        "f",
        HandlePolicy {
            ttl: Duration::from_secs(1),
            ..policy()
        },
    )
    .await
    .unwrap();
    let (b, _) = handles::renew(
        &mut tx,
        w,
        &s,
        HandleId(a.handle.id),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert!(b.expires_at > a.handle.expires_at);
    sqlx::query("UPDATE handles SET expires_at=clock_timestamp()-interval '1 second' WHERE workspace_id=$1 AND id=$2").bind(w.0).bind(a.handle.id).execute(&mut *tx).await.unwrap();
    assert!(matches!(
        handles::renew(&mut tx, w, &s, HandleId(a.handle.id), policy().ttl).await,
        Err(HandleError::Stale)
    ));
    assert!(matches!(
        handles::close(&mut tx, w, &s, HandleId(a.handle.id)).await,
        Err(HandleError::Stale)
    ));
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn two_closes_of_same_handle_do_not_deadlock() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let a = open(&mut setup, &s, "f", policy()).await.unwrap();
    setup.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    handles::close(&mut first, w, &s, HandleId(a.handle.id))
        .await
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let s2 = s.clone();
    let waiter = tokio::spawn(async move {
        let r = handles::close(&mut second, w, &s2, HandleId(a.handle.id)).await;
        second.rollback().await.unwrap();
        r
    });
    waiting(&db.pool, pid).await;
    first.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap(),
        Err(HandleError::Stale)
    ));
    invariants(&db, w).await;
}
#[tokio::test]
async fn close_and_unlink_interleave_release_once() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let a = open(&mut setup, &s, "f", policy()).await.unwrap();
    setup.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    handles::close(&mut first, w, &s, HandleId(a.handle.id))
        .await
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let waiter = tokio::spawn(async move {
        namespace::lock(&mut second, w)
            .await
            .unwrap()
            .unlink(&caller(), &path("f"))
            .await
            .unwrap();
        second.commit().await.unwrap();
    });
    waiting(&db.pool, pid).await;
    first.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn open_limit_holds_under_concurrency() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let _ = open(&mut setup, &s, "a", policy()).await.unwrap();
    let b = open(&mut setup, &s, "b", policy()).await.unwrap();
    handles::close(&mut setup, w, &s, HandleId(b.handle.id))
        .await
        .unwrap();
    setup.commit().await.unwrap();
    // Different inodes and no create avoid namespace/inode serialization: the waiter must reach advisory admission.
    let req = |name| OpenRequest {
        path: path(name),
        read: true,
        write: false,
        create: false,
        exclusive: false,
        truncate: false,
        mode: None,
    };
    let mut first = db.pool.begin().await.unwrap();
    let p = HandlePolicy {
        limit: 2,
        ..policy()
    };
    handles::open_file(&mut first, &caller(), &s, req("a"), p)
        .await
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let s2 = s.clone();
    let waiter = tokio::spawn(async move {
        let result = handles::open_file(&mut second, &caller(), &s2, req("b"), p).await;
        second.rollback().await.unwrap();
        result
    });
    waiting(&db.pool, pid).await;
    let advisory:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)").bind(pid).fetch_one(&db.pool).await.unwrap();
    assert!(advisory);
    first.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap(),
        Err(HandleError::Limit)
    ));
    invariants(&db, w).await;
}
#[tokio::test]
async fn renew_close_open_create_do_not_cycle() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let a = open(&mut setup, &s, "old", policy()).await.unwrap();

    namespace::lock(&mut setup, w)
        .await
        .unwrap()
        .unlink(&caller(), &path("old"))
        .await
        .unwrap();
    setup.commit().await.unwrap();
    let mut renewing = db.pool.begin().await.unwrap();
    handles::renew(&mut renewing, w, &s, HandleId(a.handle.id), policy().ttl)
        .await
        .unwrap();
    let mut closing = db.pool.begin().await.unwrap();
    let close_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *closing)
        .await
        .unwrap();
    let s2 = s.clone();
    let close = tokio::spawn(async move {
        let r = handles::close(&mut closing, w, &s2, HandleId(a.handle.id)).await;
        closing.commit().await.unwrap();
        r
    });
    waiting(&db.pool, close_pid).await;
    let mut creating = db.pool.begin().await.unwrap();
    let create_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *creating)
        .await
        .unwrap();
    let s3 = s.clone();
    let create = tokio::spawn(async move {
        let r = open(&mut creating, &s3, "new", policy()).await;
        creating.commit().await.unwrap();
        r
    });
    waiting(&db.pool, create_pid).await;
    let advisory:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)").bind(create_pid).fetch_one(&db.pool).await.unwrap();
    assert!(advisory);
    renewing.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), create)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), close)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let used: i64 = sqlx::query_scalar(
        "SELECT logical_inodes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(w.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(used, 1);
    invariants(&db, w).await;
}
#[tokio::test]
async fn renew_vs_unlink_at_expiry_boundary_releases_once() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    let a = open(
        &mut setup,
        &s,
        "f",
        HandlePolicy {
            ttl: Duration::from_secs(1),
            ..policy()
        },
    )
    .await
    .unwrap();
    setup.commit().await.unwrap();
    let mut renewing = db.pool.begin().await.unwrap();
    handles::renew(&mut renewing, w, &s, HandleId(a.handle.id), policy().ttl)
        .await
        .unwrap();
    let mut unlinking = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *unlinking)
        .await
        .unwrap();
    let unlink = tokio::spawn(async move {
        namespace::lock(&mut unlinking, w)
            .await
            .unwrap()
            .unlink(&caller(), &path("f"))
            .await
            .unwrap();
        unlinking.commit().await.unwrap();
    });
    waiting(&db.pool, pid).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>$1")
                .bind(a.handle.expires_at)
                .fetch_one(&db.pool)
                .await
                .unwrap();
            if expired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    renewing.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), unlink)
        .await
        .unwrap()
        .unwrap();
    let mut closing = db.pool.begin().await.unwrap();
    let (_, inode) = handles::renew(&mut closing, w, &s, HandleId(a.handle.id), policy().ttl)
        .await
        .unwrap();
    assert!(inode.charge_released_at.is_none());
    assert_eq!(inode.nlink, 0);
    handles::close(&mut closing, w, &s, HandleId(a.handle.id))
        .await
        .unwrap();
    closing.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn unlink_between_resolve_and_lock_fails_open() {
    let (db, w) = fixture().await;
    let s = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();
    open(&mut setup, &s, "f", policy()).await.unwrap();
    setup.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    let row = namespace::resolve(&mut first, w, Some(&path("f")), &caller())
        .await
        .unwrap();
    let mut other = db.pool.begin().await.unwrap();

    namespace::lock(&mut other, w)
        .await
        .unwrap()
        .unlink(&caller(), &path("f"))
        .await
        .unwrap();
    other.commit().await.unwrap();
    let rows = namespace::lock_inodes(&mut first, w, &[row.id])
        .await
        .unwrap();
    assert_eq!(rows[0].nlink, 0);
    // open_file repeats resolution and rejects the vanished path; the locked-row predicate above covers an already-resolved open.
    let r = handles::open_file(
        &mut first,
        &caller(),
        &s,
        OpenRequest {
            path: path("f"),
            read: true,
            write: false,
            create: false,
            exclusive: false,
            truncate: false,
            mode: None,
        },
        policy(),
    )
    .await;
    assert!(matches!(
        r,
        Err(HandleError::Namespace(namespace::NamespaceError::NotFound))
    ));
    first.rollback().await.unwrap();
    invariants(&db, w).await;
}

#[tokio::test]
async fn renewal_waiting_on_admission_cannot_reactivate_an_expired_slot() {
    let (db, w) = fixture().await;
    let session = actor_session(&db, w).await;
    let mut setup = db.pool.begin().await.unwrap();

    let policy = HandlePolicy {
        ttl: Duration::from_secs(60),
        limit: 1,
    };
    let req = |name: &str, create| OpenRequest {
        path: path(name),
        read: true,
        write: false,
        create,
        exclusive: false,
        truncate: false,
        mode: Some(0o600),
    };
    let old = handles::open_file(&mut setup, &caller(), &session, req("old", true), policy)
        .await
        .unwrap();
    namespace::lock(&mut setup, w)
        .await
        .unwrap()
        .create_file(&caller(), &path("replacement"), 0o600)
        .await
        .unwrap();
    let expires:time::OffsetDateTime=sqlx::query_scalar("UPDATE handles SET expires_at=clock_timestamp()+interval '2 seconds' WHERE workspace_id=$1 AND id=$2 RETURNING expires_at").bind(w.0).bind(old.handle.id).fetch_one(&mut *setup).await.unwrap();
    setup.commit().await.unwrap();
    let mut opening = db.pool.begin().await.unwrap();
    handles::lock_admission(&mut opening, session.id)
        .await
        .unwrap();
    let mut renewing = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *renewing)
        .await
        .unwrap();
    let renew_session = session.clone();
    let renewal = tokio::spawn(async move {
        let result = handles::renew(
            &mut renewing,
            w,
            &renew_session,
            HandleId(old.handle.id),
            policy.ttl,
        )
        .await;
        renewing.rollback().await.unwrap();
        result
    });
    waiting(&db.pool, pid).await;
    let advisory:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)").bind(pid).fetch_one(&db.pool).await.unwrap();
    assert!(advisory);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=$1")
                .bind(expires)
                .fetch_one(&db.pool)
                .await
                .unwrap();
            if expired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    handles::open_file(
        &mut opening,
        &caller(),
        &session,
        req("replacement", false),
        policy,
    )
    .await
    .unwrap();
    opening.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), renewal)
            .await
            .unwrap()
            .unwrap(),
        Err(HandleError::Stale)
    ));
    let mut check = db.pool.begin().await.unwrap();
    assert_eq!(
        handles::count_open(&mut check, w, session.id)
            .await
            .unwrap(),
        1
    );
    let actual: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM handles WHERE workspace_id=$1 AND id=$2")
            .bind(w.0)
            .bind(old.handle.id)
            .fetch_one(&mut *check)
            .await
            .unwrap();
    assert_eq!(actual, expires);
    check.rollback().await.unwrap();
    invariants(&db, w).await;
}
