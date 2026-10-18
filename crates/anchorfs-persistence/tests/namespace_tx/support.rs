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
pub async fn invariants(db: &Database, w: WorkspaceId) {
    assert_eq!(
        anchorfs_persistence::invariants::check(&db.pool, w)
            .await
            .unwrap(),
        Vec::<String>::new()
    );
}
