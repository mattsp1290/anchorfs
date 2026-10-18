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
