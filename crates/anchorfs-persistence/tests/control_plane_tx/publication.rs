use super::support::*;

#[tokio::test]
async fn actor_session_and_credential_expiry_during_target_wait_roll_back() {
    for credential_expiry in [false, true] {
        let db = Database::migrated().await;
        let (_, host, w) = session_fixture(&db).await;
        let mut tx = db.pool.begin().await.unwrap();
        let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
            .await
            .unwrap();
        let own = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
            .await
            .unwrap();
        let target = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        if credential_expiry {
            sqlx::query("UPDATE credentials SET expires_at=clock_timestamp()+interval '500 milliseconds' WHERE id=$1").bind(host.id.0).execute(&db.pool).await.unwrap();
        } else {
            sqlx::query("UPDATE client_sessions SET expires_at=clock_timestamp()+interval '500 milliseconds' WHERE id=$1").bind(own.id.0).execute(&db.pool).await.unwrap();
        }
        let mut actor_tx = db.pool.begin().await.unwrap();
        let actor = authority::load_actor(&mut actor_tx, &host, LockMode::Share)
            .await
            .unwrap();
        let actor_session =
            authority::load_actor_session(&mut actor_tx, &actor, w, own.id, 1, LockMode::Share)
                .await
                .unwrap();
        let mut holder = db.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM client_sessions WHERE id=$1 FOR UPDATE")
            .bind(target.id.0)
            .execute(&mut *holder)
            .await
            .unwrap();
        {
            let renew = sessions::renew(
                &mut actor_tx,
                &actor,
                &actor_session,
                w,
                target.id,
                Duration::from_secs(7200),
            );
            tokio::pin!(renew);
            assert!(
                tokio::time::timeout(Duration::from_millis(700), &mut renew)
                    .await
                    .is_err()
            );
            holder.commit().await.unwrap();
            let error = renew.await.unwrap_err();
            assert!(matches!(
                (credential_expiry, error),
                (
                    true,
                    SessionError::Authority(AuthorityError::Unauthenticated)
                ) | (false, SessionError::Authority(AuthorityError::Fenced))
            ));
        }
        actor_tx.rollback().await.unwrap();
        let expiry: time::OffsetDateTime =
            sqlx::query_scalar("SELECT expires_at FROM client_sessions WHERE id=$1")
                .bind(target.id.0)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(expiry, target.expires_at);
    }
}

#[tokio::test]
async fn mint_rechecks_target_credential_expiry_after_admission_wait() {
    let db = Database::migrated().await;
    let (admin, host, w) = session_fixture(&db).await;
    sqlx::query("UPDATE credentials SET expires_at=clock_timestamp()+interval '500 milliseconds' WHERE id=$1").bind(host.id.0).execute(&db.pool).await.unwrap();
    let mut holder = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,7001))")
        .bind(host.id.to_string())
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    {
        let create = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64);
        tokio::pin!(create);
        assert!(
            tokio::time::timeout(Duration::from_millis(700), &mut create)
                .await
                .is_err()
        );
        holder.commit().await.unwrap();
        assert!(matches!(
            create.await,
            Err(SessionError::Authority(AuthorityError::NotFound))
        ));
    }
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM client_sessions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
