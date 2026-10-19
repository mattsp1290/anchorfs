use super::support::*;
use anchorfs_core::{Identity, WorkspacePath};
use anchorfs_persistence::{
    namespace::{self, Caller, NamespaceError},
    quota::{self, Charge, QuotaError},
};
#[tokio::test]
async fn resolve_root_and_children() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    let root = namespace::resolve(&mut tx, w, None, &caller())
        .await
        .unwrap();
    assert_eq!(root.kind, "directory");
    assert!(matches!(
        namespace::resolve(
            &mut tx,
            w,
            Some(&WorkspacePath::parse("absent").unwrap()),
            &caller()
        )
        .await,
        Err(NamespaceError::NotFound)
    ));
    tx.rollback().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn resolve_hides_names_behind_denied_directory() {
    let (db, w) = fixture().await;
    sqlx::query("UPDATE inodes SET mode=448 WHERE workspace_id=$1")
        .bind(w.0)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let path = WorkspacePath::parse("secret").unwrap();
    let stranger = Caller {
        identity: Identity {
            uid: 2000,
            gid: 2000,
        },
        admin: false,
    };
    assert!(matches!(
        namespace::resolve(&mut tx, w, Some(&path), &stranger).await,
        Err(NamespaceError::Forbidden)
    ));
    assert!(matches!(
        namespace::resolve(
            &mut tx,
            w,
            Some(&path),
            &Caller {
                admin: true,
                ..stranger
            }
        )
        .await,
        Err(NamespaceError::NotFound)
    ));
    tx.rollback().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn quota_apply_rejects_over_limit_and_publishes_nothing() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    assert!(matches!(
        quota::apply(
            &mut tx,
            w,
            Charge {
                inodes: 101,
                bytes: 0
            }
        )
        .await,
        Err(QuotaError::Exceeded)
    ));
    tx.rollback().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn invariants_detect_seeded_drift() {
    let (db, w) = fixture().await;
    invariants(&db, w).await;
    sqlx::query("UPDATE inodes SET nlink=3 WHERE workspace_id=$1")
        .bind(w.0)
        .execute(&db.pool)
        .await
        .unwrap();
    let violations = anchorfs_persistence::invariants::check(&db.pool, w)
        .await
        .unwrap();
    assert!(violations.iter().any(|v| v.starts_with("nlink:")));
    sqlx::query("UPDATE inodes SET nlink=2 WHERE workspace_id=$1")
        .bind(w.0)
        .execute(&db.pool)
        .await
        .unwrap();
    invariants(&db, w).await;
}

#[tokio::test]
async fn check_workflow_predicates() {
    use anchorfs_core::{CredentialId, SessionId, sha256};
    use anchorfs_persistence::authority::{
        self, ActorCredential, ActorSession, AuthorityError, CredentialKind, CredentialRole,
        LeaseMode, WorkflowProof, WorkspaceLock,
    };
    let (db, w) = fixture().await;
    let now = time::OffsetDateTime::now_utc();
    let actor = ActorCredential {
        id: CredentialId::new(),
        workspace_id: Some(w),
        kind: CredentialKind::Host,
        role: CredentialRole::Write,
        uid: 1000,
        gid: 1000,
        epoch: 1,
        issued_by: None,
        max_reservation_bytes: 4096,
        expires_at: None,
    };
    let mut session = ActorSession {
        id: SessionId::new(),
        workspace_id: w,
        credential_id: actor.id,
        session_epoch: 1,
        credential_epoch: 1,
        bound_mutation_epoch: None,
        expires_at: now + time::Duration::hours(1),
    };
    let token = [11; 32];
    let mut lock = WorkspaceLock {
        id: w.0,
        authorization_epoch: 1,
        mutation_epoch: 2,
        state: "active".into(),
        workflow_owner_credential_id: Some(actor.id.0),
        workflow_token_hash: Some(sha256(token).0.to_vec()),
        workflow_expires_at: Some(now + time::Duration::hours(1)),
    };
    let proof = WorkflowProof { token, epoch: 2 };
    let mut tx = db.pool.begin().await.unwrap();
    assert!(matches!(
        authority::check_workflow(&mut tx, &lock, &actor, &session, None, LeaseMode::Mutation)
            .await,
        Err(AuthorityError::WorkspaceBusy)
    ));
    assert!(matches!(
        authority::check_workflow(
            &mut tx,
            &lock,
            &actor,
            &session,
            Some(&WorkflowProof {
                epoch: 1,
                ..proof.clone()
            }),
            LeaseMode::Mutation
        )
        .await,
        Err(AuthorityError::WorkspaceBusy)
    ));
    authority::check_workflow(
        &mut tx,
        &lock,
        &actor,
        &session,
        Some(&proof),
        LeaseMode::Mutation,
    )
    .await
    .unwrap();
    authority::check_workflow(&mut tx, &lock, &actor, &session, None, LeaseMode::Read)
        .await
        .unwrap();
    lock.workflow_owner_credential_id = Some(uuid::Uuid::new_v4());
    assert!(matches!(
        authority::check_workflow(
            &mut tx,
            &lock,
            &actor,
            &session,
            Some(&proof),
            LeaseMode::Mutation
        )
        .await,
        Err(AuthorityError::WorkspaceBusy)
    ));
    session.bound_mutation_epoch = Some(2);
    authority::check_workflow(
        &mut tx,
        &lock,
        &actor,
        &session,
        Some(&proof),
        LeaseMode::Mutation,
    )
    .await
    .unwrap();
    lock.workflow_expires_at = Some(now - time::Duration::hours(1));
    for p in [Some(&proof), None] {
        assert!(matches!(
            authority::check_workflow(&mut tx, &lock, &actor, &session, p, LeaseMode::Mutation)
                .await,
            Err(AuthorityError::Fenced)
        ));
    }
    authority::check_workflow(&mut tx, &lock, &actor, &session, None, LeaseMode::Read)
        .await
        .unwrap();
    session.bound_mutation_epoch = Some(1);
    for mode in [LeaseMode::Read, LeaseMode::Mutation] {
        assert!(matches!(
            authority::check_workflow(&mut tx, &lock, &actor, &session, None, mode).await,
            Err(AuthorityError::Fenced)
        ));
    }
    session.bound_mutation_epoch = None;
    assert!(matches!(
        authority::check_workflow(
            &mut tx,
            &lock,
            &actor,
            &session,
            Some(&proof),
            LeaseMode::Mutation
        )
        .await,
        Err(AuthorityError::Fenced)
    ));
    tx.rollback().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn epoch_writer_blocks_behind_inflight_mutation() {
    use anchorfs_core::{CredentialProof, SessionId};
    use anchorfs_persistence::{
        authority, credentials,
        receipts::{self, ReceiptRequest, ReceiptScope},
    };
    use std::time::Duration;
    let (db, w) = fixture().await;
    let mut setup = db.pool.begin().await.unwrap();
    let admin = credentials::bootstrap_administrator(&mut setup, 1000, 1000, 4096, None)
        .await
        .unwrap();
    let actor = authority::load_actor(
        &mut setup,
        &CredentialProof {
            id: admin.id,
            secret: admin.secret,
        },
        authority::LockMode::Share,
    )
    .await
    .unwrap();
    let host = credentials::issue(
        &mut setup,
        &actor,
        credentials::NewCredential {
            workspace_id: Some(w),
            kind: authority::CredentialKind::Host,
            role: authority::CredentialRole::Admin,
            uid: 1000,
            gid: 1000,
            expires_at: None,
            max_reservation_bytes: 4096,
        },
    )
    .await
    .unwrap();
    let proof = CredentialProof {
        id: host.id,
        secret: host.secret,
    };
    let actor = authority::load_actor(&mut setup, &proof, authority::LockMode::Share)
        .await
        .unwrap();
    let session = anchorfs_persistence::sessions::create(
        &mut setup,
        &actor,
        w,
        host.id,
        Duration::from_secs(60),
        64,
    )
    .await
    .unwrap();
    setup.commit().await.unwrap();
    let mut owner = db.pool.begin().await.unwrap();
    authority::lock_workspace(&mut owner, w)
        .await
        .unwrap()
        .unwrap();
    let actor = authority::load_actor(&mut owner, &proof, authority::LockMode::Share)
        .await
        .unwrap();
    authority::load_actor_session(
        &mut owner,
        &actor,
        w,
        SessionId(session.id.0),
        1,
        authority::LockMode::Share,
    )
    .await
    .unwrap();
    let scope = ReceiptScope::Workspace {
        workspace: w,
        credential: actor.id,
        session: session.id,
        session_epoch: 1,
        authorization_epoch: 1,
        mutation_epoch: 1,
    };
    let entry = receipts::begin(
        &mut owner,
        scope,
        ReceiptRequest {
            key: "epoch-fence",
            op_type: "directory_create",
            digest: [3; 32],
            credential_epoch: 1,
            retention: Duration::from_secs(60),
            wait_bound: Duration::from_secs(5),
        },
    )
    .await
    .unwrap();
    let receipts::ReceiptEntry::Fresh(operation) = entry else {
        panic!("fresh")
    };
    receipts::commit_result(
        &mut owner,
        scope,
        operation,
        &serde_json::json!({"committed":true}),
    )
    .await
    .unwrap();
    let mut waiter = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *waiter)
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        sqlx::query("UPDATE workspaces SET mutation_epoch=mutation_epoch+1 WHERE id=$1")
            .bind(w.0)
            .execute(&mut *waiter)
            .await
            .unwrap();
        waiter.commit().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(&db.pool).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert!(!task.is_finished());
    owner.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let epoch: i64 = sqlx::query_scalar("SELECT mutation_epoch FROM workspaces WHERE id=$1")
        .bind(w.0)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(epoch, 2);
    invariants(&db, w).await;
}
#[tokio::test]
async fn admin_host_can_be_issued_and_worker_cannot() {
    use anchorfs_core::CredentialProof;
    use anchorfs_persistence::{
        authority::{
            self, AuthorityError, CredentialKind as Kind, CredentialRole as Role, LockMode,
        },
        credentials,
    };
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    let admin = credentials::bootstrap_administrator(&mut tx, 1000, 1000, 4096, None)
        .await
        .unwrap();
    let actor = authority::load_actor(
        &mut tx,
        &CredentialProof {
            id: admin.id,
            secret: admin.secret,
        },
        LockMode::Share,
    )
    .await
    .unwrap();
    let new = |kind| credentials::NewCredential {
        workspace_id: Some(w),
        kind,
        role: Role::Admin,
        uid: 1000,
        gid: 1000,
        expires_at: None,
        max_reservation_bytes: 4096,
    };
    let host = credentials::issue(&mut tx, &actor, new(Kind::Host))
        .await
        .unwrap();
    assert!(matches!(
        credentials::issue(&mut tx, &actor, new(Kind::Worker)).await,
        Err(AuthorityError::InvalidInput)
    ));
    let host_actor = authority::load_actor(
        &mut tx,
        &CredentialProof {
            id: host.id,
            secret: host.secret,
        },
        LockMode::Share,
    )
    .await
    .unwrap();
    assert!(authority::is_workspace_admin(&host_actor));
    assert!(matches!(
        credentials::issue(&mut tx, &host_actor, new(Kind::Host)).await,
        Err(AuthorityError::Forbidden)
    ));
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
#[tokio::test]
async fn resolve_never_follows_symlinks() {
    let (db, w) = fixture().await;
    let mut tx = db.pool.begin().await.unwrap();
    let root = namespace::root(&mut tx, w).await.unwrap();
    let id:i64=sqlx::query_scalar("INSERT INTO inodes(workspace_id,kind,size,mode,uid,gid,symlink_target,atime,mtime,ctime) VALUES ($1,'symlink',7,511,1000,1000,'/opaque',now(),now(),now()) RETURNING id").bind(w.0).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO directory_entries(workspace_id,parent_inode_id,name,inode_id) VALUES ($1,$2,'sym',$3)").bind(w.0).bind(root.id).bind(id).execute(&mut *tx).await.unwrap();
    quota::apply(
        &mut tx,
        w,
        Charge {
            inodes: 1,
            bytes: 7,
        },
    )
    .await
    .unwrap();
    let row = namespace::resolve(
        &mut tx,
        w,
        Some(&WorkspacePath::parse("sym").unwrap()),
        &caller(),
    )
    .await
    .unwrap();
    assert_eq!(row.id, id);
    assert_eq!(row.symlink_target.as_deref(), Some("/opaque"));
    assert!(matches!(
        namespace::resolve(
            &mut tx,
            w,
            Some(&WorkspacePath::parse("sym/child").unwrap()),
            &caller()
        )
        .await,
        Err(NamespaceError::NotDirectory)
    ));
    assert!(
        anchorfs_persistence::invariants::check(&mut *tx, w)
            .await
            .unwrap()
            .is_empty()
    );
    tx.commit().await.unwrap();
    invariants(&db, w).await;
}
