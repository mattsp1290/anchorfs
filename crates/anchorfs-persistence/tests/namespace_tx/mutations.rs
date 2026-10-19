use super::support::*;
use anchorfs_core::{WorkspaceId, WorkspacePath};
use anchorfs_persistence::namespace::{self, NamespaceError};
use sqlx::{Postgres, Transaction};
fn path(s: &str) -> WorkspacePath {
    WorkspacePath::parse(s).unwrap()
}
async fn mkdir(tx: &mut Transaction<'_, Postgres>, w: WorkspaceId, name: &str) {
    namespace::lock(tx, w)
        .await
        .unwrap()
        .mkdir(&caller(), &path(name), 0o755)
        .await
        .unwrap();
}
#[tokio::test]
async fn mkdir_then_rmdir_restores_counts() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    mkdir(&mut tx, w, "a").await;
    assert_eq!(namespace::root(&mut tx, w).await.unwrap().nlink, 3);
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .rmdir(&caller(), &path("a"))
        .await
        .unwrap();
    assert_eq!(namespace::root(&mut tx, w).await.unwrap().nlink, 2);
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn rmdir_nonempty_rejected() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    mkdir(&mut tx, w, "a").await;
    mkdir(&mut tx, w, "a/b").await;
    assert!(matches!(
        namespace::lock(&mut tx, w)
            .await
            .unwrap()
            .rmdir(&caller(), &path("a"))
            .await,
        Err(NamespaceError::DirectoryNotEmpty)
    ));
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn link_and_unlink_track_nlink_and_symlink_bytes() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .symlink(&caller(), &path("sym"), "/opaque")
        .await
        .unwrap();
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .link(&caller(), &path("sym"), &path("alias"))
        .await
        .unwrap();
    let row = namespace::resolve(&mut tx, w, Some(&path("sym")), &caller())
        .await
        .unwrap();
    assert_eq!(row.nlink, 2);
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .unlink(&caller(), &path("sym"))
        .await
        .unwrap();
    assert_eq!(
        namespace::resolve(&mut tx, w, Some(&path("alias")), &caller())
            .await
            .unwrap()
            .nlink,
        1
    );
    let usage: (i64, i64) = sqlx::query_as(
        "SELECT logical_inodes_used,logical_bytes_used FROM workspace_quotas WHERE workspace_id=$1",
    )
    .bind(w.0)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(usage, (1, 7));
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .unlink(&caller(), &path("alias"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn rename_cycle_rejected() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    mkdir(&mut tx, w, "a").await;
    mkdir(&mut tx, w, "a/b").await;
    for (from, to) in [("a", "a/x"), ("a", "a/b/c")] {
        assert!(matches!(
            namespace::lock(&mut tx, w)
                .await
                .unwrap()
                .rename(&caller(), &path(from), &path(to))
                .await,
            Err(NamespaceError::InvalidInput)
        ));
    }
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn rename_replaces_empty_directory_same_parent() {
    replace_directory(false).await;
}
#[tokio::test]
async fn rename_replaces_empty_directory_across_parents() {
    replace_directory(true).await;
}
async fn replace_directory(across: bool) {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    for name in ["a", "b", "a/src"] {
        mkdir(&mut tx, w, name).await;
    }
    let dest = if across { "b/dst" } else { "a/dst" };
    mkdir(&mut tx, w, dest).await;
    let src = namespace::resolve(&mut tx, w, Some(&path("a/src")), &caller())
        .await
        .unwrap();
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .rename(&caller(), &path("a/src"), &path(dest))
        .await
        .unwrap();
    assert_eq!(
        namespace::resolve(&mut tx, w, Some(&path(dest)), &caller())
            .await
            .unwrap()
            .id,
        src.id
    );
    assert_eq!(
        namespace::resolve(&mut tx, w, Some(&path("a")), &caller())
            .await
            .unwrap()
            .nlink,
        if across { 2 } else { 3 }
    );
    assert_eq!(
        namespace::resolve(&mut tx, w, Some(&path("b")), &caller())
            .await
            .unwrap()
            .nlink,
        if across { 3 } else { 2 }
    );
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn rename_same_path_is_noop_but_checks_existence() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    mkdir(&mut tx, w, "a").await;
    let before = namespace::current_generation(&mut tx, w).await.unwrap();
    namespace::lock(&mut tx, w)
        .await
        .unwrap()
        .rename(&caller(), &path("a"), &path("a"))
        .await
        .unwrap();
    assert_eq!(
        namespace::current_generation(&mut tx, w).await.unwrap(),
        before
    );
    assert!(matches!(
        namespace::lock(&mut tx, w)
            .await
            .unwrap()
            .rename(&caller(), &path("missing"), &path("missing"))
            .await,
        Err(NamespaceError::NotFound)
    ));
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn concurrent_mkdir_same_name_one_exists() {
    concurrent(false).await;
}
#[tokio::test]
async fn concurrent_sibling_cycle_one_wins() {
    concurrent(true).await;
}
async fn concurrent(cycle: bool) {
    use std::time::Duration;
    let (db, w) = fixture().await;
    let mut setup = db.pool.begin().await.unwrap();
    if cycle {
        mkdir(&mut setup, w, "x").await;
        mkdir(&mut setup, w, "y").await;
    }
    let before = namespace::current_generation(&mut setup, w).await.unwrap();
    setup.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    namespace::lock(&mut first, w).await.unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let waiter = tokio::spawn(async move {
        namespace::lock(&mut second, w).await.unwrap();
        let result = if cycle {
            namespace::lock(&mut second, w)
                .await
                .unwrap()
                .rename(&caller(), &path("y"), &path("x/y"))
                .await
        } else {
            namespace::lock(&mut second, w)
                .await
                .unwrap()
                .mkdir(&caller(), &path("a"), 0o755)
                .await
        };
        second.rollback().await.unwrap();
        result
    });
    tokio::time::timeout(Duration::from_secs(5),async {loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(&db.pool).await.unwrap();
        if waiting {break;}tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    if cycle {
        namespace::lock(&mut first, w)
            .await
            .unwrap()
            .rename(&caller(), &path("x"), &path("y/x"))
            .await
            .unwrap();
    } else {
        namespace::lock(&mut first, w)
            .await
            .unwrap()
            .mkdir(&caller(), &path("a"), 0o755)
            .await
            .unwrap();
    }
    first.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    if cycle {
        assert!(matches!(result, Err(NamespaceError::NotFound)));
    } else {
        assert!(matches!(result, Err(NamespaceError::Exists)));
    }
    let mut tx = db.pool.begin().await.unwrap();
    assert_eq!(
        namespace::current_generation(&mut tx, w).await.unwrap(),
        before + 1
    );
    if cycle {
        let row = namespace::resolve(&mut tx, w, Some(&path("y/x")), &caller())
            .await
            .unwrap();
        assert_eq!(
            namespace::ancestors(&mut tx, w, row.id)
                .await
                .unwrap()
                .len(),
            3
        );
    }
    tx.rollback().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn collation_is_byte_order() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    for name in ["B", "a", "é", "_", "Z"] {
        namespace::lock(&mut tx, w)
            .await
            .unwrap()
            .symlink(&caller(), &path(name), "opaque")
            .await
            .unwrap();
    }
    let root = namespace::root(&mut tx, w).await.unwrap();
    let rows = namespace::list(&mut tx, w, root.id, None, 10)
        .await
        .unwrap();
    assert_eq!(
        rows.into_iter().map(|(name, _)| name).collect::<Vec<_>>(),
        ["B", "Z", "_", "a", "é"]
    );
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}

#[tokio::test]
async fn mutation_context_bumps_namespace_once() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    let before = namespace::current_generation(&mut tx, w).await.unwrap();
    {
        let mut namespace = namespace::lock(&mut tx, w).await.unwrap();
        assert_eq!(namespace.generation(), before);
        namespace.mkdir(&caller(), &path("a"), 0o755).await.unwrap();
        namespace.mkdir(&caller(), &path("b"), 0o755).await.unwrap();
        assert_eq!(namespace.generation(), before + 1);
    }
    assert_eq!(
        namespace::current_generation(&mut tx, w).await.unwrap(),
        before + 1
    );
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
