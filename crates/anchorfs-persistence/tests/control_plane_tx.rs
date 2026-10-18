//! Synthetic acceptance cases derived from the approved Stage 1a plan.
mod common;
use anchorfs_core::{CredentialProof, WorkspaceId};
use anchorfs_persistence::{
    authority::{
        self, ActorCredential, AuthorityError, CredentialKind as Kind, CredentialRole as Role,
        LockMode,
    },
    credentials::{self, BootstrapError, NewCredential},
    workspaces,
};
use common::Database;
use sqlx::{Postgres, Transaction};

async fn administrator(db: &Database) -> CredentialProof {
    let mut tx = db.pool.begin().await.unwrap();
    let issued = credentials::bootstrap_administrator(&mut tx, 1000, 1000, 4096, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    CredentialProof {
        id: issued.id,
        secret: issued.secret,
    }
}
fn new_credential(workspace: Option<WorkspaceId>, kind: Kind) -> NewCredential {
    NewCredential {
        workspace_id: workspace,
        kind,
        role: if kind == Kind::Administrator {
            Role::Admin
        } else {
            Role::Write
        },
        uid: 1000,
        gid: 1000,
        expires_at: None,
        max_reservation_bytes: 4096,
    }
}
async fn workspace(tx: &mut Transaction<'_, Postgres>) -> WorkspaceId {
    WorkspaceId(
        workspaces::create(
            tx,
            workspaces::NewWorkspace {
                logical_bytes_limit: 1000,
                logical_inodes_limit: 100,
                physical_bytes_limit: 2000,
                staging_bytes_limit: 1000,
                root_uid: 1000,
                root_gid: 1000,
                root_mode: 0o755,
            },
        )
        .await
        .unwrap()
        .id,
    )
}
async fn issue(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    new: NewCredential,
) -> CredentialProof {
    let issued = credentials::issue(tx, actor, new).await.unwrap();
    CredentialProof {
        id: issued.id,
        secret: issued.secret,
    }
}
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

use anchorfs_core::SessionId;
use anchorfs_persistence::sessions::{self, SessionError};
use std::time::Duration;
async fn session_fixture(db: &Database) -> (CredentialProof, CredentialProof, WorkspaceId) {
    let admin = administrator(db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    let w = workspace(&mut tx).await;
    let host = issue(&mut tx, &actor, new_credential(Some(w), Kind::Host)).await;
    tx.commit().await.unwrap();
    (admin, host, w)
}
#[tokio::test]
async fn session_limit_counts_only_active_and_rotation_frees_slots() {
    let db = Database::migrated().await;
    let (admin, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &admin, LockMode::Share)
        .await
        .unwrap();
    for _ in 0..64 {
        sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
            .await
            .unwrap();
    }
    for expired in [true, false] {
        sqlx::query("INSERT INTO client_sessions (id,workspace_id,credential_id,credential_epoch,session_epoch,expires_at,revoked_at) VALUES ($1,$2,$3,1,1,CASE WHEN $4 THEN now()-interval '1 second' ELSE now()+interval '1 hour' END,CASE WHEN $4 THEN NULL ELSE now() END)")
            .bind(SessionId::new().0).bind(w.0).bind(host.id.0).bind(expired).execute(&mut *tx).await.unwrap();
    }
    assert_eq!(sessions::count_active(&mut tx, host.id).await.unwrap(), 64);
    assert!(matches!(
        sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64).await,
        Err(SessionError::Limit)
    ));
    sqlx::query("UPDATE client_sessions SET expires_at=now()-interval '1 second' WHERE id=(SELECT id FROM client_sessions WHERE credential_id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp() LIMIT 1)").bind(host.id.0).execute(&mut *tx).await.unwrap();
    sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
        .await
        .unwrap();
    credentials::rotate(&mut tx, &actor, host.id).await.unwrap();
    assert_eq!(sessions::count_active(&mut tx, host.id).await.unwrap(), 0);
    let session = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
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
        sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
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
    let first = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
        .await
        .unwrap();
    let second = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
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

#[tokio::test]
async fn own_session_renew_with_update_lock_has_no_deadlock() {
    use anchorfs_persistence::receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope};
    let db = Database::migrated().await;
    let (_, host, w) = session_fixture(&db).await;
    let mut tx = db.pool.begin().await.unwrap();
    let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
        .await
        .unwrap();
    let session = sessions::create(&mut tx, &actor, w, host.id, Duration::from_secs(3600), 64)
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
    let ReceiptEntry::Fresh(id) = receipts::begin(&mut owner, scope, request()).await.unwrap()
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
    let a_session = sessions::create(&mut tx, &actor, w, a.id, Duration::from_secs(3600), 64)
        .await
        .unwrap();
    let b_session = sessions::create(&mut tx, &actor, w, b.id, Duration::from_secs(3600), 64)
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
