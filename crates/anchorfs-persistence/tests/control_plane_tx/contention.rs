use super::support::*;

#[tokio::test]
async fn cross_actor_mutations_surface_40p01() {
    let db = Database::migrated().await;
    let a = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &a, LockMode::Share)
        .await
        .unwrap();
    let b = issue(&mut tx, &actor, new_credential(None, Kind::Administrator)).await;
    tx.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    let first_actor = authority::load_actor(&mut first, &a, LockMode::Share)
        .await
        .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let second_actor = authority::load_actor(&mut second, &b, LockMode::Share)
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = barrier.clone();
    let b_id = b.id;
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        let result = credentials::rotate(&mut first, &first_actor, b_id).await;
        if result.is_ok() {
            first.commit().await.unwrap();
        } else {
            first.rollback().await.unwrap();
        }
        result
    });
    let a_id = a.id;
    let second = tokio::spawn(async move {
        barrier.wait().await;
        let result = credentials::rotate(&mut second, &second_actor, a_id).await;
        if result.is_ok() {
            second.commit().await.unwrap();
        } else {
            second.rollback().await.unwrap();
        }
        result
    });
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    let (failure, issued, target) = match (first, second) {
        (Err(error), Ok(issued)) => (error, issued, b.id),
        (Ok(issued), Err(error)) => (error, issued, a.id),
        _ => panic!("exactly one deadlock victim expected"),
    };
    let AuthorityError::Database(error) = failure else {
        panic!("database error expected");
    };
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("40P01")
    );
    // The winning rotation changed the victim's proof. Reload its new authority before retry.
    let proof = CredentialProof {
        id: issued.id,
        secret: issued.secret,
    };
    let mut retry = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut retry, &proof, LockMode::Share)
        .await
        .unwrap();
    credentials::rotate(&mut retry, &actor, target)
        .await
        .unwrap();
    retry.commit().await.unwrap();
}

#[tokio::test]
async fn own_session_renew_with_update_lock_has_no_deadlock() {
    use anchorfs_persistence::receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope};
    let db = Database::migrated().await;
    let (_, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
        .await
        .unwrap();
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
    tx.commit().await.unwrap();
    let request = || ReceiptRequest {
        key: "self-renew",
        op_type: "session_renew",
        digest: [7; 32],
        credential_epoch: 1,
        retention: Duration::from_secs(3600),
        wait_bound: Duration::from_secs(5),
    };
    let scope = ReceiptScope::Workspace {
        workspace: w,
        credential: host.id,
        session: session.id,
        session_epoch: 1,
        authorization_epoch: 1,
        mutation_epoch: 1,
    };
    let mut owner = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut owner, &host, LockMode::Share)
        .await
        .unwrap();
    let actor_session =
        authority::load_actor_session(&mut owner, &actor, w, session.id, 1, LockMode::Update)
            .await
            .unwrap();
    let ReceiptEntry::Fresh {
        operation_id: id, ..
    } = receipts::begin(&mut owner, scope, request()).await.unwrap()
    else {
        panic!("fresh receipt expected");
    };
    let renewed = sessions::renew(
        &mut owner,
        &actor,
        &actor_session,
        w,
        session.id,
        Duration::from_secs(3600),
    )
    .await
    .unwrap();
    let result =
        serde_json::json!({"expires_at":renewed.expires_at.unix_timestamp_nanos().to_string()});
    receipts::commit_result(&mut owner, scope, id, &result)
        .await
        .unwrap();
    let pool = db.pool.clone();
    let waiter = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
            .await
            .unwrap();
        authority::load_actor_session(&mut tx, &actor, w, session.id, 1, LockMode::Update)
            .await
            .unwrap();
        let entry = receipts::begin(&mut tx, scope, request()).await.unwrap();
        tx.commit().await.unwrap();
        entry
    });
    let mut waiter = waiter;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut waiter)
            .await
            .is_err()
    );
    owner.commit().await.unwrap();
    assert!(
        matches!(waiter.await.unwrap(),ReceiptEntry::Replay {operation_id,result:r} if operation_id==id && r==result)
    );
}

#[tokio::test]
async fn cross_session_renew_surfaces_40p01() {
    let db = Database::migrated().await;
    let (admin, a, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    let b = issue(&mut tx, &actor, new_credential(Some(w), Kind::Host)).await;
    let a_session = sessions::create(
        &mut tx,
        &actor,
        w,
        a.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    let b_session = sessions::create(
        &mut tx,
        &actor,
        w,
        b.id,
        Duration::from_secs(3600),
        64,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut first = db.pool.begin().await.unwrap();
    let first_actor = authority::load_actor(&mut first, &a, LockMode::Share)
        .await
        .unwrap();
    let first_session = authority::load_actor_session(
        &mut first,
        &first_actor,
        w,
        a_session.id,
        1,
        LockMode::Share,
    )
    .await
    .unwrap();
    let mut second = db.pool.begin().await.unwrap();
    let second_actor = authority::load_actor(&mut second, &b, LockMode::Share)
        .await
        .unwrap();
    let second_session = authority::load_actor_session(
        &mut second,
        &second_actor,
        w,
        b_session.id,
        1,
        LockMode::Share,
    )
    .await
    .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first_barrier = barrier.clone();
    let first = tokio::spawn(async move {
        first_barrier.wait().await;
        let result = sessions::renew(
            &mut first,
            &first_actor,
            &first_session,
            w,
            b_session.id,
            Duration::from_secs(3600),
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
        let result = sessions::renew(
            &mut second,
            &second_actor,
            &second_session,
            w,
            a_session.id,
            Duration::from_secs(3600),
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
    assert_eq!(results.iter().filter(|r| matches!(r,Err(SessionError::Database(e)) if e.as_database_error().and_then(|e| e.code()).as_deref()==Some("40P01"))).count(),1);
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &a, LockMode::Share)
        .await
        .unwrap();
    let actor_session =
        authority::load_actor_session(&mut tx, &actor, w, a_session.id, 1, LockMode::Share)
            .await
            .unwrap();
    sessions::renew(
        &mut tx,
        &actor,
        &actor_session,
        w,
        b_session.id,
        Duration::from_secs(3600),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn sibling_revocation_during_parent_cascade_surfaces_40p01() {
    let db = Database::migrated().await;
    let root = administrator(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let root_actor = authority::load_actor(&mut tx, &root, LockMode::Share)
        .await
        .unwrap();
    let parent = issue(
        &mut tx,
        &root_actor,
        new_credential(None, Kind::Administrator),
    )
    .await;
    let parent_actor = authority::load_actor(&mut tx, &parent, LockMode::Share)
        .await
        .unwrap();
    let child_a = issue(
        &mut tx,
        &parent_actor,
        new_credential(None, Kind::Administrator),
    )
    .await;
    let child_b = issue(
        &mut tx,
        &parent_actor,
        new_credential(None, Kind::Administrator),
    )
    .await;
    tx.commit().await.unwrap();
    let (small, large) = if child_a.id < child_b.id {
        (child_a, child_b)
    } else {
        (child_b, child_a)
    };
    let mut sibling_tx = db.pool.begin().await.unwrap();
    let sibling_actor = authority::load_actor(&mut sibling_tx, &large, LockMode::Share)
        .await
        .unwrap();
    let mut cascade_tx = db.pool.begin().await.unwrap();
    let root_actor = authority::load_actor(&mut cascade_tx, &root, LockMode::Share)
        .await
        .unwrap();
    // Pause the top-down cascade after locking its parent and first ascending child.
    for id in [parent.id, small.id] {
        sqlx::query("SELECT id FROM credentials WHERE id=$1 FOR UPDATE")
            .bind(id.0)
            .execute(&mut *cascade_tx)
            .await
            .unwrap();
    }
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let sibling_barrier = barrier.clone();
    let target = small.id;
    let sibling = tokio::spawn(async move {
        sibling_barrier.wait().await;
        let result = credentials::revoke(&mut sibling_tx, &sibling_actor, target).await;
        if result.is_ok() {
            sibling_tx.commit().await.unwrap();
        } else {
            sibling_tx.rollback().await.unwrap();
        }
        result
    });
    let cascade = tokio::spawn(async move {
        barrier.wait().await;
        let result = credentials::revoke(&mut cascade_tx, &root_actor, parent.id).await;
        if result.is_ok() {
            cascade_tx.commit().await.unwrap();
        } else {
            cascade_tx.rollback().await.unwrap();
        }
        result
    });
    let (sibling, cascade) = (sibling.await.unwrap(), cascade.await.unwrap());
    let error = match (&sibling, &cascade) {
        (Err(AuthorityError::Database(e)), Ok(_)) | (Ok(_), Err(AuthorityError::Database(e))) => e,
        _ => panic!("one deadlock victim expected"),
    };
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("40P01")
    );
    let mut retry = db.pool.begin().await.unwrap();
    if cascade.is_err() {
        let actor = authority::load_actor(&mut retry, &root, LockMode::Share)
            .await
            .unwrap();
        credentials::revoke(&mut retry, &actor, parent.id)
            .await
            .unwrap();
        retry.commit().await.unwrap();
    } else {
        // The successful cascade revoked the other actor: its retry is correctly fenced.
        assert!(matches!(
            authority::load_actor(&mut retry, &large, LockMode::Share).await,
            Err(AuthorityError::Unauthenticated)
        ));
    }
}
