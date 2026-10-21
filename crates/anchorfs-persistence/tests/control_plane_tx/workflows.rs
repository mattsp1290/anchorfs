//! Synthetic schedules from approved specs 02/03/07 and the Stage 2a plan.
use super::support::*;
use anchorfs_persistence::{
    authority::{ActorSession, LeaseMode, WorkflowProof, WorkspaceLock},
    invariants,
    workflows::{self, WorkflowError},
};

struct LeaseFixture {
    db: Database,
    admin: CredentialProof,
    workspace: WorkspaceId,
    host: CredentialProof,
    actor: ActorCredential,
    session: ActorSession,
}
impl LeaseFixture {
    async fn new() -> Self {
        let db = Database::migrated().await;
        let (admin, host, workspace) = session_fixture(&db).await;
        let mut tx = db.pool.begin().await.unwrap();
        let actor = authority::load_actor(&mut tx, &host, LockMode::Share)
            .await
            .unwrap();
        let s = sessions::create(
            &mut tx,
            &actor,
            workspace,
            host.id,
            Duration::from_secs(60),
            32,
            None,
        )
        .await
        .unwrap();
        let session =
            authority::load_actor_session(&mut tx, &actor, workspace, s.id, 1, LockMode::Share)
                .await
                .unwrap();
        tx.commit().await.unwrap();
        Self {
            db,
            admin,
            workspace,
            host,
            actor,
            session,
        }
    }
    async fn lock(&self, tx: &mut Transaction<'_, Postgres>) -> WorkspaceLock {
        authority::lock_workspace_for_update(tx, self.workspace)
            .await
            .unwrap()
            .unwrap()
    }
    async fn acquire(&self) -> workflows::LeaseGrant {
        let mut tx = self.db.pool.begin().await.unwrap();
        let lock = self.lock(&mut tx).await;
        let g = workflows::acquire(
            &mut tx,
            &lock,
            &self.actor,
            &self.session,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        g
    }
    async fn expire(&self) {
        sqlx::query("UPDATE workspaces SET workflow_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(self.workspace.0).execute(&self.db.pool).await.unwrap();
    }
    async fn invariants(&self) {
        assert!(
            invariants::check(&self.db.pool, self.workspace)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            invariants::check_deployment(&self.db.pool)
                .await
                .unwrap()
                .is_empty()
        );
    }
    async fn second_host(&self) -> ActorCredential {
        let mut tx = self.db.pool.begin().await.unwrap();
        let admin = authority::load_actor(&mut tx, &self.admin, LockMode::Share)
            .await
            .unwrap();
        let proof = issue(
            &mut tx,
            &admin,
            new_credential(Some(self.workspace), Kind::Host),
        )
        .await;
        let actor = authority::load_actor(&mut tx, &proof, LockMode::Share)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        actor
    }
}
fn proof(g: &workflows::LeaseGrant) -> WorkflowProof {
    WorkflowProof {
        token: anchorfs_core::decode_hex_32(g.token.expose()).unwrap(),
        epoch: g.mutation_epoch,
    }
}
async fn set_timeout(tx: &mut Transaction<'_, Postgres>, ms: u32) {
    sqlx::query("SELECT set_config('lock_timeout',$1,true)")
        .bind(format!("{ms}ms"))
        .execute(&mut **tx)
        .await
        .unwrap();
}
async fn wait_lock(pool: &sqlx::PgPool) {
    tokio::time::timeout(Duration::from_secs(5),async { loop { let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND state='active')").fetch_one(pool).await.unwrap();if waiting {break;}tokio::time::sleep(Duration::from_millis(10)).await; }}).await.unwrap();
}
#[tokio::test]
async fn acquire_release_takeover_transitions() {
    let f = LeaseFixture::new().await;
    let g = f.acquire().await;
    assert_eq!(g.mutation_epoch, 2);
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let renewed = workflows::renew(
        &mut tx,
        &lock,
        &f.actor,
        &f.session,
        &proof(&g),
        Duration::from_secs(120),
    )
    .await
    .unwrap();
    assert_eq!(renewed.mutation_epoch, 2);
    assert!(renewed.expires_at > g.expires_at);
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert_eq!(
        workflows::release(&mut tx, &lock, &f.actor, &f.session, &proof(&g))
            .await
            .unwrap(),
        3
    );
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(lock.workflow_owner_credential_id.is_none());
    assert!(lock.workflow_token_hash.is_none());
    assert!(lock.workflow_expires_at.is_none());
    tx.commit().await.unwrap();
    let g = f.acquire().await;
    assert_eq!(g.mutation_epoch, 4);
    f.expire().await;
    let b = f.second_host().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let g2 = workflows::takeover(&mut tx, &lock, &b, &f.session, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(g2.mutation_epoch, 5);
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            &proof(&g),
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::Authority(AuthorityError::Forbidden))
    ));
    assert!(matches!(
        authority::check_workflow(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            Some(&proof(&g)),
            LeaseMode::Mutation
        )
        .await,
        Err(AuthorityError::WorkspaceBusy)
    ));
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn acquire_rejects_live_and_dead_leases() {
    let f = LeaseFixture::new().await;
    f.acquire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::acquire(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::LeaseActive)
    ));
    tx.rollback().await.unwrap();
    f.expire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::acquire(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::LeaseExpiredBlocksAcquire)
    ));
    workflows::takeover(
        &mut tx,
        &lock,
        &f.actor,
        &f.session,
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn renew_release_require_owner_and_live_proof() {
    let f = LeaseFixture::new().await;
    let g = f.acquire().await;
    let b = f.second_host().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &b,
            &f.session,
            &proof(&g),
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::Authority(AuthorityError::Forbidden))
    ));
    let mut wrong = proof(&g);
    wrong.epoch += 1;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            &wrong,
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::ProofMismatch)
    ));
    tx.rollback().await.unwrap();
    f.expire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &f.actor,
            &f.session,
            &proof(&g),
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::LeaseExpiredFencesProof)
    ));
    assert!(matches!(
        workflows::release(&mut tx, &lock, &f.actor, &f.session, &proof(&g)).await,
        Err(WorkflowError::LeaseExpiredFencesProof)
    ));
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn worker_read_role_and_bound_sessions_cannot_hold_leases() {
    let f = LeaseFixture::new().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    for kind in [Kind::Worker, Kind::Administrator] {
        let mut actor = f.actor.clone();
        actor.kind = kind;
        assert!(matches!(
            workflows::acquire(&mut tx, &lock, &actor, &f.session, Duration::from_secs(60)).await,
            Err(WorkflowError::Authority(AuthorityError::Forbidden))
        ));
    }
    let mut actor = f.actor.clone();
    actor.role = Role::Read;
    assert!(matches!(
        workflows::acquire(&mut tx, &lock, &actor, &f.session, Duration::from_secs(60)).await,
        Err(WorkflowError::Authority(AuthorityError::Forbidden))
    ));
    let mut session = f.session.clone();
    session.bound_mutation_epoch = Some(1);
    assert!(matches!(
        workflows::acquire(&mut tx, &lock, &f.actor, &session, Duration::from_secs(60)).await,
        Err(WorkflowError::SessionBound)
    ));
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn concurrent_acquires_serialize_without_deadlock() {
    let f = LeaseFixture::new().await;
    let options = f.db.pool.connect_options();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect_with((*options).clone())
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let barrier = barrier.clone();
        let actor = f.actor.clone();
        let session = f.session.clone();
        let ws = f.workspace;
        tasks.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            set_timeout(&mut tx, 5000).await;
            barrier.wait().await;
            let lock = authority::lock_workspace_for_update(&mut tx, ws)
                .await
                .unwrap()
                .unwrap();
            let result =
                workflows::acquire(&mut tx, &lock, &actor, &session, Duration::from_secs(60)).await;
            tx.commit().await.unwrap();
            result
        }));
    }
    let mut wins = 0;
    let mut losses = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => wins += 1,
            Err(WorkflowError::LeaseActive) => losses += 1,
            Err(e) => panic!("unexpected {e:?}"),
        }
    }
    assert_eq!((wins, losses), (1, 7));
    f.invariants().await;
}
#[tokio::test]
async fn release_fences_a_session_bound_after_lock_wait() {
    let f = LeaseFixture::new().await;
    let g = f.acquire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    workflows::release(&mut tx, &lock, &f.actor, &f.session, &proof(&g))
        .await
        .unwrap();
    let pool = f.db.pool.clone();
    let actor = f.actor.clone();
    let ws = f.workspace;
    let token = g.token.expose().clone();
    let task = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let lock = authority::lock_workspace(&mut tx, ws)
            .await
            .unwrap()
            .unwrap();
        sessions::create(
            &mut tx,
            &actor,
            ws,
            actor.id,
            Duration::from_secs(60),
            32,
            Some((&lock, &token)),
        )
        .await
    });
    wait_lock(&f.db.pool).await;
    tx.commit().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SessionError::Workflow(WorkflowError::LeaseAbsent))
    ));
    f.invariants().await;
}
#[tokio::test]
async fn bind_then_release_leaves_bound_session_fenced() {
    let f = LeaseFixture::new().await;
    let g = f.acquire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = authority::lock_workspace(&mut tx, f.workspace)
        .await
        .unwrap()
        .unwrap();
    let bound = sessions::create(
        &mut tx,
        &f.actor,
        f.workspace,
        f.actor.id,
        Duration::from_secs(60),
        32,
        Some((&lock, g.token.expose())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    workflows::release(&mut tx, &lock, &f.actor, &f.session, &proof(&g))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let session =
        authority::load_actor_session(&mut tx, &f.actor, f.workspace, bound.id, 1, LockMode::Share)
            .await
            .unwrap();
    for mode in [LeaseMode::Mutation, LeaseMode::Read] {
        assert!(matches!(
            authority::check_workflow(&mut tx, &lock, &f.actor, &session, Some(&proof(&g)), mode)
                .await,
            Err(AuthorityError::Fenced)
        ));
    }
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn invariant_detects_bound_epoch_above_workspace() {
    let f = LeaseFixture::new().await;
    sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=2 WHERE id=$1")
        .bind(f.session.id.0)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let violations = invariants::check(&f.db.pool, f.workspace).await.unwrap();
    assert!(violations.contains(&format!("bound-epoch:{}", f.session.id)));
    sqlx::query("UPDATE client_sessions SET bound_mutation_epoch=NULL WHERE id=$1")
        .bind(f.session.id.0)
        .execute(&f.db.pool)
        .await
        .unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn lock_upgrade_from_share_is_documented() {
    let f = LeaseFixture::new().await;
    let mut a = f.db.pool.begin().await.unwrap();
    let mut b = f.db.pool.begin().await.unwrap();
    authority::lock_workspace(&mut a, f.workspace)
        .await
        .unwrap();
    authority::lock_workspace(&mut b, f.workspace)
        .await
        .unwrap();
    set_timeout(&mut a, 200).await;
    let error = sqlx::query("UPDATE workspaces SET mutation_epoch=mutation_epoch+1 WHERE id=$1")
        .bind(f.workspace.0)
        .execute(&mut *a)
        .await
        .unwrap_err();
    assert_eq!(error.as_database_error().unwrap().code().unwrap(), "55P03");
    a.rollback().await.unwrap();
    b.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn unauthenticated_acquire_does_not_block_readers() {
    let f = LeaseFixture::new().await;
    let mut holder = f.db.pool.begin().await.unwrap();
    authority::lock_workspace(&mut holder, f.workspace)
        .await
        .unwrap();
    let mut bad = CredentialProof {
        id: f.host.id,
        secret: anchorfs_core::Secret::new("00".repeat(32)),
    };
    let mut tx = f.db.pool.begin().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_millis(100),
            authority::preauthenticate(&mut tx, &bad, f.workspace)
        )
        .await
        .unwrap(),
        Err(AuthorityError::Unauthenticated)
    ));
    tx.rollback().await.unwrap();
    bad.secret = anchorfs_core::Secret::new(f.host.secret.expose().clone());
    sqlx::query("UPDATE credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(bad.id.0)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_millis(100),
            authority::preauthenticate(&mut tx, &bad, f.workspace)
        )
        .await
        .unwrap(),
        Err(AuthorityError::Unauthenticated)
    ));
    tx.rollback().await.unwrap();
    holder.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn worker_with_token_and_no_lease_is_forbidden() {
    let f = LeaseFixture::new().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let mut worker = f.actor.clone();
    worker.kind = Kind::Worker;
    assert!(matches!(
        sessions::create(
            &mut tx,
            &worker,
            f.workspace,
            worker.id,
            Duration::from_secs(60),
            32,
            Some((&lock, &"00".repeat(32)))
        )
        .await,
        Err(SessionError::Authority(AuthorityError::Forbidden))
    ));
    assert!(matches!(
        sessions::create(
            &mut tx,
            &f.actor,
            f.workspace,
            anchorfs_core::CredentialId::new(),
            Duration::from_secs(60),
            32,
            Some((&lock, &"00".repeat(32)))
        )
        .await,
        Err(SessionError::Authority(AuthorityError::NotFound))
    ));
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn non_owner_renew_release_on_dead_or_free_lease() {
    let f = LeaseFixture::new().await;
    let b = f.second_host().await;
    let g = f.acquire().await;
    f.expire().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &b,
            &f.session,
            &proof(&g),
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::Authority(AuthorityError::Forbidden))
    ));
    assert!(matches!(
        workflows::release(&mut tx, &lock, &b, &f.session, &proof(&g)).await,
        Err(WorkflowError::Authority(AuthorityError::Forbidden))
    ));
    tx.rollback().await.unwrap();
    sqlx::query("UPDATE workspaces SET workflow_owner_credential_id=NULL,workflow_token_hash=NULL,workflow_expires_at=NULL WHERE id=$1").bind(f.workspace.0).execute(&f.db.pool).await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    assert!(matches!(
        workflows::renew(
            &mut tx,
            &lock,
            &b,
            &f.session,
            &proof(&g),
            Duration::from_secs(60)
        )
        .await,
        Err(WorkflowError::LeaseAbsent)
    ));
    assert!(matches!(
        workflows::release(&mut tx, &lock, &b, &f.session, &proof(&g)).await,
        Err(WorkflowError::LeaseAbsent)
    ));
    tx.rollback().await.unwrap();
    f.invariants().await;
}
#[tokio::test]
async fn lease_wait_past_deadline_does_not_commit() {
    let f = LeaseFixture::new().await;
    for timeout in [200, 5000] {
        let mut holder = f.db.pool.begin().await.unwrap();
        authority::lock_workspace(&mut holder, f.workspace)
            .await
            .unwrap();
        let pool = f.db.pool.clone();
        let actor = f.actor.clone();
        let session = f.session.clone();
        let ws = f.workspace;
        let task = tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            set_timeout(&mut tx, timeout).await;
            let lock = authority::lock_workspace_for_update(&mut tx, ws).await?;
            let g = workflows::acquire(
                &mut tx,
                &lock.unwrap(),
                &actor,
                &session,
                Duration::from_secs(60),
            )
            .await?;
            tx.commit().await?;
            Ok::<_, WorkflowError>(g)
        });
        wait_lock(&f.db.pool).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        holder.commit().await.unwrap();
        let result = task.await.unwrap();
        if timeout == 200 {
            assert!(matches!(result, Err(WorkflowError::Database(_))));
            let recorded: bool = sqlx::query_scalar(
                "SELECT workflow_expires_at IS NOT NULL FROM workspaces WHERE id=$1",
            )
            .bind(ws.0)
            .fetch_one(&f.db.pool)
            .await
            .unwrap();
            assert!(!recorded);
        } else {
            result.unwrap();
        }
    }
    f.invariants().await;
}
#[tokio::test]
async fn acquire_under_publication_load() {
    let f = LeaseFixture::new().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect_with((*f.db.pool.connect_options()).clone())
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(5));
    let start = std::time::Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let pool = pool.clone();
        let ws = f.workspace;
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let mut first=true;
            while start.elapsed()<Duration::from_secs(5) {
                let mut tx=pool.begin().await.unwrap();
                authority::lock_workspace(&mut tx,ws).await.unwrap();
                if first { first=false;barrier.wait().await; }
                sqlx::query("UPDATE inodes SET atime=clock_timestamp() WHERE workspace_id=$1 AND kind='directory'").bind(ws.0).execute(&mut *tx).await.unwrap();
                tx.commit().await.unwrap();
                tokio::task::yield_now().await;
            }
        }));
    }
    // All four publishers hold shared locks before starting the exclusive wait.
    barrier.wait().await;
    let wait_start = std::time::Instant::now();
    let mut tx = pool.begin().await.unwrap();
    set_timeout(&mut tx, 10000).await;
    match authority::lock_workspace_for_update(&mut tx, f.workspace).await {
        Ok(Some(lock)) => {
            workflows::acquire(
                &mut tx,
                &lock,
                &f.actor,
                &f.session,
                Duration::from_secs(60),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            println!(
                "acquire_under_publication_load elapsed_ms={}",
                wait_start.elapsed().as_millis()
            );
        }
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("55P03") => {
            tx.rollback().await.unwrap();
            println!(
                "exclusive lease operations can starve under continuous share traffic; hosts retry"
            );
        }
        other => panic!("unexpected load result {other:?}"),
    }
    for task in tasks {
        task.await.unwrap();
    }
    f.invariants().await;
}
#[tokio::test]
async fn any_host_may_rebind_a_session_bound_to_another_hosts_lease() {
    let f = LeaseFixture::new().await;
    let g = f.acquire().await;
    let b = f.second_host().await;
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let s = sessions::create(
        &mut tx,
        &f.actor,
        f.workspace,
        f.actor.id,
        Duration::from_secs(60),
        32,
        Some((&lock, g.token.expose())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = f.db.pool.begin().await.unwrap();
    let lock = f.lock(&mut tx).await;
    let rebound = sessions::rebind(
        &mut tx,
        &b,
        &f.session,
        f.workspace,
        s.id,
        Duration::from_secs(60),
        32,
        None,
    )
    .await
    .unwrap();
    assert!(rebound.bound_mutation_epoch.is_none());
    let session = authority::load_actor_session(
        &mut tx,
        &f.actor,
        f.workspace,
        s.id,
        rebound.session_epoch,
        LockMode::Share,
    )
    .await
    .unwrap();
    assert!(matches!(
        authority::check_workflow(
            &mut tx,
            &lock,
            &f.actor,
            &session,
            None,
            LeaseMode::Mutation
        )
        .await,
        Err(AuthorityError::WorkspaceBusy)
    ));
    tx.commit().await.unwrap();
    f.invariants().await;
}
