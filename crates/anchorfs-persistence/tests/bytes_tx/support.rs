use crate::common::Database;
use anchorfs_core::{Identity, WorkspaceId};
use anchorfs_persistence::{
    namespace::Caller,
    workspaces::{self, NewWorkspace},
};
pub fn caller() -> Caller {
    Caller {
        identity: Identity {
            uid: 1000,
            gid: 1000,
        },
        admin: false,
    }
}
pub async fn fixture() -> (Database, WorkspaceId) {
    let db = Database::migrated().await;
    let mut tx = db.pool.begin().await.unwrap();
    let w = workspaces::create(
        &mut tx,
        NewWorkspace {
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
    .unwrap();
    tx.commit().await.unwrap();
    (db, WorkspaceId(w.id))
}
pub async fn actor_session(
    db: &Database,
    w: WorkspaceId,
) -> anchorfs_persistence::authority::ActorSession {
    use anchorfs_persistence::{
        authority::{self, CredentialKind, CredentialRole, LockMode},
        credentials, sessions,
    };
    let mut tx = db.pool.begin().await.unwrap();
    let admin = credentials::bootstrap_administrator(&mut tx, 1000, 1000, 4096, None)
        .await
        .unwrap();
    let actor = authority::load_actor(
        &mut tx,
        &anchorfs_core::CredentialProof {
            id: admin.id,
            secret: admin.secret,
        },
        LockMode::Share,
    )
    .await
    .unwrap();
    let issued = credentials::issue(
        &mut tx,
        &actor,
        credentials::NewCredential {
            workspace_id: Some(w),
            kind: CredentialKind::Host,
            role: CredentialRole::Write,
            uid: 1000,
            gid: 1000,
            expires_at: None,
            max_reservation_bytes: 4096,
        },
    )
    .await
    .unwrap();
    let actor = authority::load_actor(
        &mut tx,
        &anchorfs_core::CredentialProof {
            id: issued.id,
            secret: issued.secret,
        },
        LockMode::Share,
    )
    .await
    .unwrap();
    let grant = sessions::create(
        &mut tx,
        &actor,
        w,
        actor.id,
        std::time::Duration::from_secs(3600),
        64,
    )
    .await
    .unwrap();
    let session = authority::load_actor_session(&mut tx, &actor, w, grant.id, 1, LockMode::Share)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    session
}
pub async fn operation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session: &anchorfs_persistence::authority::ActorSession,
) -> anchorfs_core::OperationId {
    use anchorfs_persistence::receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope};
    let id = anchorfs_core::OperationId::new();
    let entry = receipts::begin(
        tx,
        ReceiptScope::Workspace {
            workspace: session.workspace_id,
            credential: session.credential_id,
            session: session.id,
            session_epoch: session.session_epoch,
            authorization_epoch: 1,
            mutation_epoch: 1,
        },
        ReceiptRequest {
            key: &id.to_string(),
            op_type: "write",
            digest: [1; 32],
            credential_epoch: 1,
            retention: std::time::Duration::from_secs(3600),
            wait_bound: std::time::Duration::from_secs(1),
        },
    )
    .await
    .unwrap();
    match entry {
        ReceiptEntry::Fresh(id) => id,
        _ => panic!("fresh operation expected"),
    }
}

pub async fn actor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session: &anchorfs_persistence::authority::ActorSession,
) -> anchorfs_persistence::authority::ActorCredential {
    let row: anchorfs_persistence::rows::CredentialRow =
        sqlx::query_as("SELECT * FROM credentials WHERE id=$1")
            .bind(session.credential_id.0)
            .fetch_one(&mut **tx)
            .await
            .unwrap();
    row.try_into().unwrap()
}
