use super::support::*;

#[tokio::test]
async fn session_limit_counts_only_active_and_rotation_frees_slots() {
    let db = Database::migrated().await;
    let (admin, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    for _ in 0..64 {
        sessions::create(
            &mut tx,
            &actor,
            w,
            host.id,
            Duration::from_secs(3600),
            64,
            None,
        )
        .await
        .unwrap();
    }
    for expired in [true, false] {
        sqlx::query("INSERT INTO client_sessions (id,workspace_id,credential_id,credential_epoch,session_epoch,expires_at,revoked_at) VALUES ($1,$2,$3,1,1,CASE WHEN $4 THEN now()-interval '1 second' ELSE now()+interval '1 hour' END,CASE WHEN $4 THEN NULL ELSE now() END)")
            .bind(SessionId::new().0).bind(w.0).bind(host.id.0).bind(expired).execute(&mut *tx).await.unwrap();
    }
    assert_eq!(sessions::count_active(&mut tx, host.id).await.unwrap(), 64);
    assert!(matches!(
        sessions::create(
            &mut tx,
            &actor,
            w,
            host.id,
            Duration::from_secs(3600),
            64,
            None
        )
        .await,
        Err(SessionError::Limit)
    ));
    sqlx::query("UPDATE client_sessions SET expires_at=now()-interval '1 second' WHERE id=(SELECT id FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp() LIMIT 1)").bind(host.id.0).execute(&mut *tx).await.unwrap();
    sessions::create(
        &mut tx,
        &actor,
        w,
        host.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    credentials::rotate(&mut tx, &actor, host.id).await.unwrap();
    assert_eq!(sessions::count_active(&mut tx, host.id).await.unwrap(), 0);
    let session = sessions::create(
        &mut tx,
        &actor,
        w,
        host.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    assert_eq!(session.credential_epoch, 2);
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn session_limit_holds_under_concurrency() {
    let db = Database::migrated().await;
    let (admin, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    for _ in 0..63 {
        sessions::create(
            &mut tx,
            &actor,
            w,
            host.id,
            Duration::from_secs(3600),
            64,
            None,
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    let first_actor = authority::load_actor(&mut first, &admin, LockMode::Share)
        .await
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let second_actor = authority::load_actor(&mut second, &admin, LockMode::Share)
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = barrier.clone();
    let target = host.id;
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        let result = sessions::create(
            &mut first,
            &first_actor,
            w,
            target,
            Duration::from_secs(3600),
            64,
            None,
        )
        .await;
        if result.is_ok() {
            first.commit().await.unwrap();
        } else {
            first.rollback().await.unwrap();
        }
        result
    });
    let second = tokio::spawn(async move {
        barrier.wait().await;
        let result = sessions::create(
            &mut second,
            &second_actor,
            w,
            target,
            Duration::from_secs(3600),
            64,
            None,
        )
        .await;
        if result.is_ok() {
            second.commit().await.unwrap();
        } else {
            second.rollback().await.unwrap();
        }
        result
    });
    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(SessionError::Limit)))
            .count(),
        1
    );
    let mut tx = db.pool.begin().await.unwrap();
    assert_eq!(sessions::count_active(&mut tx, host.id).await.unwrap(), 64);
}

#[tokio::test]
async fn rebind_increments_epoch_and_clears_binding_and_fences_old_proof() {
    let db = Database::migrated().await;
    let (_, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
        .await
        .unwrap();
    let first = sessions::create(
        &mut tx,
        &actor,
        w,
        host.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    let second = sessions::create(
        &mut tx,
        &actor,
        w,
        host.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    let actor_session =
        authority::load_actor_session(&mut tx, &actor, w, first.id, 1, LockMode::Share)
            .await
            .unwrap();
    sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=1,expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(second.id.0).execute(&mut *tx).await.unwrap();
    assert!(matches!(
        authority::load_actor_session(&mut tx, &actor, w, second.id, 1, LockMode::Share).await,
        Err(AuthorityError::Fenced)
    ));
    let rebound = sessions::rebind(
        &mut tx,
        &actor,
        &actor_session,
        w,
        second.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    assert_eq!(rebound.session_epoch, 2);
    assert_eq!(rebound.bound_mutation_epoch, None);
    assert!(matches!(
        authority::load_actor_session(&mut tx, &actor, w, second.id, 1, LockMode::Share).await,
        Err(AuthorityError::Fenced)
    ));
    authority::load_actor_session(&mut tx, &actor, w, second.id, 2, LockMode::Share)
        .await
        .unwrap();
    let mut wrong_actor = actor.clone();
    wrong_actor.epoch += 1;
    assert!(matches!(
        authority::load_actor_session(&mut tx, &wrong_actor, w, second.id, 2, LockMode::Share)
            .await,
        Err(AuthorityError::Fenced)
    ));
    wrong_actor.id = anchorfs_core::CredentialId::new();
    assert!(matches!(
        authority::load_actor_session(&mut tx, &wrong_actor, w, second.id, 2, LockMode::Share)
            .await,
        Err(AuthorityError::Unauthenticated)
    ));
    tx.commit().await.unwrap();
}
