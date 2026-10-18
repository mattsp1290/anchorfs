use super::support::*;

#[tokio::test]
async fn bootstrap_twice_fails() {
    let db = Database::migrated().await;
    administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    assert!(matches!(
        credentials::bootstrap_administrator(&mut tx, 1, 2, 4096, None).await,
        Err(BootstrapError::AdministratorExists)
    ));
}

#[tokio::test]
async fn create_workspace_rows() {
    let db = Database::migrated().await;
    let mut tx = db.pool.begin().await.unwrap();
    let id = workspace(&mut tx).await;
    tx.commit().await.unwrap();
    let count: i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM workspaces WHERE id=$1)+(SELECT count(*) FROM workspace_quotas WHERE workspace_id=$1)+(SELECT count(*) FROM namespace_locks WHERE workspace_id=$1)+(SELECT count(*) FROM inodes WHERE workspace_id=$1)").bind(id.0).fetch_one(&db.pool).await.unwrap();
    assert_eq!(count, 4);
    let root: (i64, i32, i32) =
        sqlx::query_as("SELECT generation,nlink,mode FROM inodes WHERE workspace_id=$1")
            .bind(id.0)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(root, (1, 2, 0o755));
}
