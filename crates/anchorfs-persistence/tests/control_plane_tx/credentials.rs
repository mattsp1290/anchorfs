use super::support::*;

#[tokio::test]
async fn issue_rotate_revoke_cascade() {
    let db = Database::migrated().await;
    let proof = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &proof, LockMode::Share)
        .await
        .unwrap();
    let w = workspace(&mut tx).await;
    let host = issue(&mut tx, &actor, new_credential(Some(w), Kind::Host)).await;
    let host_actor = authority::load_actor(&mut tx, &host, LockMode::Share)
        .await
        .unwrap();
    let worker = issue(&mut tx, &host_actor, new_credential(Some(w), Kind::Worker)).await;
    let session = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO client_sessions (id,workspace_id,credential_id,credential_epoch,session_epoch,expires_at) VALUES ($1,$2,$3,1,1,now()+interval '1 hour')").bind(session).bind(w.0).bind(worker.id.0).execute(&mut *tx).await.unwrap();
    let rotated = credentials::rotate(&mut tx, &actor, worker.id)
        .await
        .unwrap();
    assert_eq!(rotated.epoch, 2);
    assert!(matches!(
        authority::load_actor(&mut tx, &worker, LockMode::Share).await,
        Err(AuthorityError::Unauthenticated)
    ));
    let revoked = credentials::revoke(&mut tx, &actor, host.id).await.unwrap();
    assert_eq!(revoked.revoked_credentials.len(), 2);
    tx.commit().await.unwrap();
    let epochs: Vec<i64> =
        sqlx::query_scalar("SELECT epoch FROM credentials WHERE id=ANY($1) ORDER BY epoch")
            .bind(vec![host.id.0, worker.id.0])
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(epochs, vec![2, 3]);
    let session_revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM client_sessions WHERE id=$1")
            .bind(session)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(session_revoked);
}

#[tokio::test]
async fn revoke_including_actor_commits() {
    let db = Database::migrated().await;
    let a = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &a, LockMode::Share)
        .await
        .unwrap();
    let b = issue(&mut tx, &actor, new_credential(None, Kind::Administrator)).await;
    tx.commit().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        credentials::is_ancestor_or_self(&mut tx, a.id, b.id)
            .await
            .unwrap()
    );
    let actor = authority::load_actor(&mut tx, &b, LockMode::Update)
        .await
        .unwrap();
    credentials::revoke(&mut tx, &actor, a.id).await.unwrap();
    tx.commit().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    for proof in [&a, &b] {
        assert!(matches!(
            authority::load_actor(&mut tx, proof, LockMode::Share).await,
            Err(AuthorityError::Unauthenticated)
        ));
    }
    credentials::bootstrap_administrator(&mut tx, 1, 2, 4096, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn issue_bounds_host_lifetime_and_allowance() {
    let db = Database::migrated().await;
    let proof = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &proof, LockMode::Share)
        .await
        .unwrap();
    let w = workspace(&mut tx).await;
    let expiry: time::OffsetDateTime =
        sqlx::query_scalar("SELECT clock_timestamp()+interval '1 hour'")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let mut new = new_credential(Some(w), Kind::Host);
    new.expires_at = Some(expiry);
    let host = issue(&mut tx, &actor, new).await;
    let host = authority::load_actor(&mut tx, &host, LockMode::Share)
        .await
        .unwrap();
    for (expires_at, allowance) in [
        (None, 4096),
        (Some(expiry + time::Duration::SECOND), 4096),
        (Some(expiry), 4097),
    ] {
        let mut new = new_credential(Some(w), Kind::Worker);
        new.expires_at = expires_at;
        new.max_reservation_bytes = allowance;
        assert!(matches!(
            credentials::issue(&mut tx, &host, new).await,
            Err(AuthorityError::Forbidden)
        ));
    }
    let mut new = new_credential(Some(w), Kind::Worker);
    new.expires_at = Some(expiry);
    credentials::issue(&mut tx, &host, new).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn revoke_cascade_catches_concurrent_issuance() {
    let db = Database::migrated().await;
    let proof = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let admin = authority::load_actor(&mut tx, &proof, LockMode::Share)
        .await
        .unwrap();
    let w = workspace(&mut tx).await;
    let host = issue(&mut tx, &admin, new_credential(Some(w), Kind::Host)).await;
    tx.commit().await.unwrap();
    let mut issuing = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut issuing, &host, LockMode::Share)
        .await
        .unwrap();
    let worker = issue(&mut issuing, &actor, new_credential(Some(w), Kind::Worker)).await;
    let mut revoking = db.pool.begin().await.unwrap();
    let admin = authority::load_actor(&mut revoking, &proof, LockMode::Share)
        .await
        .unwrap();
    {
        let revoke = credentials::revoke(&mut revoking, &admin, host.id);
        tokio::pin!(revoke);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut revoke)
                .await
                .is_err()
        );
        issuing.commit().await.unwrap();
        let result = revoke.await.unwrap();
        assert!(result.revoked_credentials.contains(&worker.id));
    }
    revoking.commit().await.unwrap();
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM credentials WHERE id=$1")
            .bind(worker.id.0)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(revoked);
}
