use super::support::*;

#[tokio::test]
async fn expired_rebind_shares_admission_with_create_and_other_rebinds() {
    for other_rebind in [false, true] {
        let db = Database::migrated().await;
        let (admin, host, w) = session_fixture(&db).await;
        let mut tx = db.pool.begin().await.unwrap();
        let admin = authority::load_actor(&mut tx, &admin, LockMode::Share)
            .await
            .unwrap();
        let worker = issue(&mut tx, &admin, new_credential(Some(w), Kind::Worker)).await;
        let host_actor = authority::load_actor(&mut tx, &host, LockMode::Share)
            .await
            .unwrap();
        let actor_session = sessions::create(
            &mut tx,
            &host_actor,
            w,
            host.id,
            Duration::from_secs(3600),
            64,
            None,
        )
        .await
        .unwrap();
        let mut targets = Vec::new();
        for _ in 0..2 {
            let target = sessions::create(
                &mut tx,
                &admin,
                w,
                worker.id,
                Duration::from_secs(3600),
                64,
                None,
            )
            .await
            .unwrap();
            sqlx::query("UPDATE client_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(target.id.0).execute(&mut *tx).await.unwrap();
            targets.push(target.id);
        }
        sessions::create(
            &mut tx,
            &admin,
            w,
            worker.id,
            Duration::from_secs(3600),
            2,
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mut tasks = Vec::new();
        for (i, target) in targets.into_iter().enumerate() {
            let pool = db.pool.clone();
            let proof = host.clone();
            let barrier = barrier.clone();
            let actor_id = actor_session.id;
            let worker_id = worker.id;
            tasks.push(tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                let actor = authority::load_actor(&mut tx, &proof, LockMode::Share)
                    .await
                    .unwrap();
                let actor_session =
                    authority::load_actor_session(&mut tx, &actor, w, actor_id, 1, LockMode::Share)
                        .await
                        .unwrap();
                barrier.wait().await;
                let result = if i == 0 || other_rebind {
                    sessions::rebind(
                        &mut tx,
                        &actor,
                        &actor_session,
                        w,
                        target,
                        Duration::from_secs(3600),
                        2,
                        None,
                    )
                    .await
                } else {
                    sessions::create(
                        &mut tx,
                        &actor,
                        w,
                        worker_id,
                        Duration::from_secs(3600),
                        2,
                        None,
                    )
                    .await
                };
                if result.is_ok() {
                    tx.commit().await.unwrap();
                } else {
                    tx.rollback().await.unwrap();
                }
                result
            }));
        }
        let mut succeeded = 0;
        let mut limited = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => succeeded += 1,
                Err(SessionError::Limit) => limited += 1,
                other => panic!("unexpected result {other:?}"),
            }
        }
        assert_eq!((succeeded, limited), (1, 1));
        let mut tx = db.pool.begin().await.unwrap();
        assert_eq!(sessions::count_active(&mut tx, worker.id).await.unwrap(), 2);
    }
}

#[tokio::test]
async fn renewed_session_does_not_release_a_hidden_mvcc_slot() {
    let db = Database::migrated().await;
    let (admin, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let admin_actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    let worker = issue(&mut tx, &admin_actor, new_credential(Some(w), Kind::Worker)).await;
    let own = sessions::create(
        &mut tx,
        &admin_actor,
        w,
        host.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    let target = sessions::create(
        &mut tx,
        &admin_actor,
        w,
        worker.id,
        Duration::from_secs(3600),
        2,
        None,
    )
    .await
    .unwrap();
    sessions::create(
        &mut tx,
        &admin_actor,
        w,
        worker.id,
        Duration::from_secs(3600),
        2,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let old_expiry:time::OffsetDateTime=sqlx::query_scalar("UPDATE client_sessions SET expires_at=clock_timestamp()+interval '700 milliseconds' WHERE id=$1 RETURNING expires_at").bind(target.id.0).fetch_one(&db.pool).await.unwrap();
    let mut renewing = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut renewing, &host, LockMode::Share)
        .await
        .unwrap();
    let actor_session =
        authority::load_actor_session(&mut renewing, &actor, w, own.id, 1, LockMode::Share)
            .await
            .unwrap();
    sessions::renew(
        &mut renewing,
        &actor,
        &actor_session,
        w,
        target.id,
        Duration::from_secs(3600),
    )
    .await
    .unwrap();
    // Let the committed version expire while the extension is still uncommitted.
    loop {
        let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=$1")
            .bind(old_expiry)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        if expired {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pool = db.pool.clone();
    let worker_id = worker.id;
    let mut minter = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
            .await
            .unwrap();
        let result = sessions::create(
            &mut tx,
            &actor,
            w,
            worker_id,
            Duration::from_secs(3600),
            2,
            None,
        )
        .await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
        result
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut minter)
            .await
            .is_err()
    );
    renewing.commit().await.unwrap();
    assert!(matches!(minter.await.unwrap(), Err(SessionError::Limit)));
    let mut tx = db.pool.begin().await.unwrap();
    assert_eq!(sessions::count_active(&mut tx, worker.id).await.unwrap(), 2);
}
