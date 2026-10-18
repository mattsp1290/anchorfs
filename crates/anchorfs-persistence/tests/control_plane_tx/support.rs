//! Synthetic acceptance cases derived from the approved Stage 1a plan.
pub(super) use crate::common::Database;
pub(super) use anchorfs_core::{CredentialProof, WorkspaceId};
pub(super) use anchorfs_persistence::{
    authority::{
        self, ActorCredential, AuthorityError, CredentialKind as Kind, CredentialRole as Role,
        LockMode,
    },
    credentials::{self, BootstrapError, NewCredential},
    workspaces,
};
pub(super) use sqlx::{Postgres, Transaction};

pub(super) async fn administrator(db: &Database) -> CredentialProof {
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
pub(super) fn new_credential(workspace: Option<WorkspaceId>, kind: Kind) -> NewCredential {
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
pub(super) async fn workspace(tx: &mut Transaction<'_, Postgres>) -> WorkspaceId {
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
pub(super) async fn issue(
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

pub(super) use anchorfs_core::SessionId;
pub(super) use anchorfs_persistence::sessions::{self, SessionError};
pub(super) use std::time::Duration;
pub(super) async fn session_fixture(
    db: &Database,
) -> (CredentialProof, CredentialProof, WorkspaceId) {
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
