//! Synthetic fixtures authored from the approved WP3 schema, run only in disposable containers.
use anchorfs_core::{DeploymentId, Secret};
use anchorfs_persistence::{
    meta, migrate,
    pool::{self, PoolConfig},
    rows,
    settings::{self, DurabilityProfile, DurabilitySettings},
};
use sqlx::PgPool;
use std::time::Duration;
use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

// Observed with `docker pull postgres:17` on 2026-10-17. Multi-platform official image manifest.
const POSTGRES_IMAGE_DIGEST: &str =
    "sha256:2d2b8998d31037bf721cfdf764d76ba74171b4fab3431b7f72c27c56ddbdf9e3";
const EXPECTED_TABLES: &[&str] = &[
    "schema_meta",
    "deployment_quotas",
    "admin_operations",
    "workspaces",
    "workspace_quotas",
    "namespace_locks",
    "credentials",
    "client_sessions",
    "inodes",
    "directory_entries",
    "upload_reservations",
    "objects",
    "manifests",
    "manifest_chunks",
    "operations",
    "handles",
    "read_pins",
    "deletion_candidates",
];
struct Database {
    _container: ContainerAsync<Postgres>,
    pool: PgPool,
    config: PoolConfig,
}
impl Database {
    async fn start(args: &[&str]) -> Self {
        // The module defaults to fsync=off; explicitly enable the official server's durable defaults.
        let container = Postgres::default()
            .with_fsync_enabled()
            .with_tag(format!("17@{POSTGRES_IMAGE_DIGEST}"))
            .with_cmd(args.iter().copied())
            .start()
            .await
            .expect("Docker and disposable PostgreSQL are required");
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let config = PoolConfig {
            url: Secret::new(format!(
                "postgres://postgres:postgres@{host}:{port}/postgres"
            )),
            max_connections: 4,
            acquire_timeout: Duration::from_secs(15),
        };
        let pool = pool::connect(&config).await.unwrap();
        Self {
            _container: container,
            pool,
            config,
        }
    }
    async fn migrated() -> Self {
        let db = Self::start(&[]).await;
        migrate::apply(&db.pool).await.unwrap();
        meta::initialize(
            &db.pool,
            &DeploymentId::parse("test-deployment").unwrap(),
            &quotas(),
            anchorfs_core::Limits::default().chunk_size,
        )
        .await
        .unwrap();
        db
    }
}
fn quotas() -> meta::DeploymentQuotaConfig {
    meta::DeploymentQuotaConfig {
        physical_bytes_limit: 1_000_000,
        staging_bytes_limit: 100_000,
    }
}
fn assert_db(error: sqlx::Error, code: &str, constraint: &str) {
    let error = error.as_database_error().expect("database error");
    assert_eq!(error.code().as_deref(), Some(code), "{error}");
    assert_eq!(error.constraint(), Some(constraint), "{error}");
}

// All SQL values are independently selected synthetic fixtures. UUIDs are random and inode IDs
// allocated from the schema sequence so two workspaces cannot accidentally share fixture inodes.
struct Fixture {
    workspace: Uuid,
    host: Uuid,
    worker: Uuid,
    session: Uuid,
    file: i64,
    manifest: Uuid,
}
async fn fixture(pool: &PgPool) -> Fixture {
    let f = Fixture {
        workspace: Uuid::new_v4(),
        host: Uuid::new_v4(),
        worker: Uuid::new_v4(),
        session: Uuid::new_v4(),
        file: sqlx::query_scalar("SELECT nextval('inodes_id_seq')")
            .fetch_one(pool)
            .await
            .unwrap(),
        manifest: Uuid::new_v4(),
    };
    let root: i64 = sqlx::query_scalar("SELECT nextval('inodes_id_seq')")
        .fetch_one(pool)
        .await
        .unwrap();
    let administrator = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let reservation = Uuid::new_v4();
    let object = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO credentials(id,kind,role,uid,gid,secret_hash,max_reservation_bytes,created_at) VALUES ($1,'administrator','admin',1000,1000,decode(repeat('ab',32),'hex'),4096,now())").bind(administrator).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,root_inode_id,state,created_at,updated_at) VALUES ($1,$2,'active',now(),now())").bind(f.workspace).bind(root).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO workspace_quotas(workspace_id,logical_bytes_limit,logical_inodes_limit,physical_bytes_limit,staging_bytes_limit) VALUES ($1,10000,100,10000,10000)").bind(f.workspace).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO namespace_locks(workspace_id) VALUES ($1)")
        .bind(f.workspace)
        .execute(&mut *tx)
        .await
        .unwrap();
    for (id, kind, issuer) in [
        (f.host, "host", administrator),
        (f.worker, "worker", f.host),
    ] {
        sqlx::query("INSERT INTO credentials(id,workspace_id,kind,role,issued_by_credential_id,uid,gid,secret_hash,max_reservation_bytes,expires_at,created_at) VALUES ($1,$2,$3,'write',$4,1000,1000,decode(repeat('ab',32),'hex'),4096,now()+interval '1 hour',now())").bind(id).bind(f.workspace).bind(kind).bind(issuer).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("INSERT INTO client_sessions(id,workspace_id,credential_id,credential_epoch,session_epoch,expires_at) VALUES ($1,$2,$3,1,1,now()+interval '1 hour')").bind(f.session).bind(f.workspace).bind(f.host).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO admin_operations(id,credential_id,credential_epoch_at_start,idempotency_key,op_type,request_digest,owner_token,heartbeat_at,state,created_at,expires_at) VALUES ($1,$2,1,'admin-key','workspace_create',decode(repeat('ab',32),'hex'),decode(repeat('cd',32),'hex'),now(),'committed',now(),now()+interval '1 day')").bind(Uuid::new_v4()).bind(administrator).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO operations(workspace_id,id,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,created_at,expires_at) VALUES ($1,$2,$3,$4,1,1,1,1,decode(repeat('cd',32),'hex'),now(),'write-key','write',decode(repeat('ab',32),'hex'),'committed',now(),now()+interval '1 day')").bind(f.workspace).bind(operation).bind(f.host).bind(f.session).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO upload_reservations(workspace_id,id,operation_id,credential_id,reserved_bytes,state,heartbeat_at,expires_at) VALUES ($1,$2,$3,$4,4096,'consumed',now(),now()+interval '1 hour')").bind(f.workspace).bind(reservation).bind(operation).bind(f.host).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO objects(workspace_id,id,byte_length,sha256,state,reservation_id,uploaded_at,published_at) VALUES ($1,$2,4,decode(repeat('ab',32),'hex'),'published',$3,now(),now())").bind(f.workspace).bind(object).bind(reservation).execute(&mut *tx).await.unwrap();
    for (id, kind, current) in [
        (root, "directory", None),
        (f.file, "file", Some(f.manifest)),
    ] {
        sqlx::query("INSERT INTO inodes(workspace_id,id,kind,mode,uid,gid,atime,mtime,ctime,current_manifest_id) VALUES ($1,$2,$3,493,1000,1000,now(),now(),now(),$4)").bind(f.workspace).bind(id).bind(kind).bind(current).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("INSERT INTO manifests(workspace_id,id,inode_id,size,chunk_size,created_by_operation_id,created_at,generation) VALUES ($1,$2,$3,4,4096,$4,now(),1)").bind(f.workspace).bind(f.manifest).bind(f.file).bind(operation).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO manifest_chunks(workspace_id,manifest_id,chunk_index,chunk_size,object_id,sha256,logical_length) VALUES ($1,$2,0,4096,$3,decode(repeat('ab',32),'hex'),4)").bind(f.workspace).bind(f.manifest).bind(object).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO directory_entries(workspace_id,parent_inode_id,name,inode_id) VALUES ($1,$2,'ok',$3)").bind(f.workspace).bind(root).bind(f.file).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO handles(workspace_id,id,inode_id,client_session_id,credential_id,access_read,access_write,opened_at,expires_at) VALUES ($1,$2,$3,$4,$5,true,true,now(),now()+interval '1 hour')").bind(f.workspace).bind(Uuid::new_v4()).bind(f.file).bind(f.session).bind(f.host).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO read_pins(workspace_id,id,manifest_id,client_session_id,expires_at) VALUES ($1,$2,$3,$4,now()+interval '1 hour')").bind(f.workspace).bind(Uuid::new_v4()).bind(f.manifest).bind(f.session).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO deletion_candidates(workspace_id,object_id,reason,physical_bytes,recorded_at) VALUES ($1,$2,'synthetic-positive-control',4,now())").bind(f.workspace).bind(object).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    f
}

async fn assert_tables(pool: &PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for name in EXPECTED_TABLES {
        assert!(tables.iter().any(|t| t == name), "missing {name}");
    }
    assert_eq!(tables.len(), EXPECTED_TABLES.len() + 1);
}
#[tokio::test]
async fn migrations_apply_revert_reapply() {
    let db = Database::start(&[]).await;
    assert!(!migrate::database_is_newer(&db.pool).await.unwrap());
    migrate::apply(&db.pool).await.unwrap();
    assert!(matches!(
        meta::read_versions(&db.pool).await,
        Err(meta::Error::MetadataRowCount(0))
    ));
    meta::initialize(
        &db.pool,
        &DeploymentId::parse("test-deployment").unwrap(),
        &quotas(),
        anchorfs_core::Limits::default().chunk_size,
    )
    .await
    .unwrap();
    migrate::apply(&db.pool).await.unwrap();
    assert_tables(&db.pool).await;
    fixture(&db.pool).await;
    migrate::revert_all(&db.pool).await.unwrap();
    let relations:Vec<String>=sqlx::query_scalar("SELECT relname FROM pg_class WHERE relnamespace='public'::regnamespace AND relkind IN ('r','S','i') ORDER BY relname").fetch_all(&db.pool).await.unwrap();
    assert_eq!(relations, vec!["_sqlx_migrations", "_sqlx_migrations_pkey"]);
    let constraint_names:Vec<String>=sqlx::query_scalar("SELECT conname FROM pg_constraint WHERE connamespace='public'::regnamespace ORDER BY conname").fetch_all(&db.pool).await.unwrap();
    assert_eq!(constraint_names, vec!["_sqlx_migrations_pkey"]);
    for query in [
        "SELECT count(*) FROM pg_type WHERE typnamespace='public'::regnamespace AND typtype='e'",
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relnamespace='public'::regnamespace",
        "SELECT count(*) FROM pg_proc WHERE pronamespace='public'::regnamespace",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            0,
            "{query}"
        );
    }
    migrate::apply(&db.pool).await.unwrap();
    let deployment = DeploymentId::parse("test-deployment").unwrap();
    meta::initialize(
        &db.pool,
        &deployment,
        &quotas(),
        anchorfs_core::Limits::default().chunk_size,
    )
    .await
    .unwrap();
    assert_tables(&db.pool).await;
    let versions = meta::read_versions(&db.pool).await.unwrap();
    assert_eq!(
        versions,
        meta::Versions {
            schema_version: 1,
            object_format_version: 1
        }
    );
    meta::check_versions(&versions).unwrap();
    for bad in [
        meta::Versions {
            schema_version: 2,
            object_format_version: 1,
        },
        meta::Versions {
            schema_version: 1,
            object_format_version: 2,
        },
    ] {
        assert!(meta::check_versions(&bad).is_err());
    }
    assert!(matches!(
        meta::initialize(
            &db.pool,
            &DeploymentId::parse("another").unwrap(),
            &quotas(),
            anchorfs_core::Limits::default().chunk_size,
        )
        .await,
        Err(meta::Error::DeploymentMismatch { .. })
    ));
    meta::initialize(
        &db.pool,
        &deployment,
        &meta::DeploymentQuotaConfig {
            physical_bytes_limit: 2,
            staging_bytes_limit: 3,
        },
        anchorfs_core::Limits::default().chunk_size,
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT physical_bytes_limit FROM deployment_quotas")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        quotas().physical_bytes_limit
    );
}
#[tokio::test]
async fn fixture_transaction_commits() {
    let db = Database::migrated().await;
    let f = fixture(&db.pool).await;
    // Table identifiers come exclusively from the closed EXPECTED_TABLES constant above.
    for name in EXPECTED_TABLES {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {name}")))
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(count > 0, "positive control for {name}");
    }
    let workspace: rows::WorkspaceRow = sqlx::query_as("SELECT * FROM workspaces WHERE id=$1")
        .bind(f.workspace)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(anchorfs_core::WorkspaceId::from(&workspace).0, f.workspace);
    let credential: rows::CredentialRow = sqlx::query_as("SELECT * FROM credentials WHERE id=$1")
        .bind(f.host)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(anchorfs_core::CredentialId::from(&credential).0, f.host);
    let meta: rows::SchemaMetaRow = sqlx::query_as("SELECT * FROM schema_meta")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        DeploymentId::try_from(&meta).unwrap().as_str(),
        "test-deployment"
    );
}
#[tokio::test]
async fn deferred_constraints_checked_at_commit() {
    let db = Database::migrated().await;
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,root_inode_id,state,created_at,updated_at) VALUES ($1,987654321,'active',now(),now())").bind(Uuid::new_v4()).execute(&mut *tx).await.expect("INSERT must accept deferred reference");
    assert_db(
        tx.commit().await.unwrap_err(),
        "23503",
        "workspaces_root_inode_fk",
    );
}

#[tokio::test]
async fn constraints_enforced() {
    let db = Database::migrated().await;
    let f = fixture(&db.pool).await;
    let other = fixture(&db.pool).await;
    // UPDATE reuses a committed valid row, changing exactly the field under test.
    for (query, code, name) in [
        (
            "UPDATE inodes SET mode=3584 WHERE kind='file'",
            "23514",
            "inodes_mode_check",
        ),
        (
            "UPDATE directory_entries SET name='..'",
            "23514",
            "directory_entries_name_check",
        ),
        (
            "UPDATE workspaces SET workflow_expires_at=now()",
            "23514",
            "workspaces_workflow_all_or_none",
        ),
        (
            "INSERT INTO schema_meta SELECT * FROM schema_meta",
            "23505",
            "schema_meta_pkey",
        ),
        (
            "INSERT INTO manifest_chunks SELECT workspace_id,manifest_id,chunk_index+1,chunk_size,object_id,sha256,chunk_size+1 FROM manifest_chunks",
            "23514",
            "manifest_chunks_logical_length_check",
        ),
        (
            "INSERT INTO credentials(id,kind,role,uid,gid,secret_hash,max_reservation_bytes,created_at) VALUES(gen_random_uuid(),'host','write',1000,1000,decode(repeat('ab',32),'hex'),4096,now())",
            "23514",
            "credentials_scope_check",
        ),
        (
            "UPDATE credentials SET role='admin' WHERE kind='worker'",
            "23514",
            "credentials_worker_role_check",
        ),
    ] {
        assert_db(
            sqlx::query(query).execute(&db.pool).await.unwrap_err(),
            code,
            name,
        );
    }
    let error = sqlx::query("INSERT INTO credentials(id,workspace_id,kind,role,issued_by_credential_id,uid,gid,secret_hash,max_reservation_bytes,created_at) VALUES(gen_random_uuid(),$2,'worker','write',$1,1000,1000,decode(repeat('ab',32),'hex'),4096,now())")
        .bind(other.host)
        .bind(f.workspace)
        .execute(&db.pool)
        .await
        .unwrap_err();
    assert_db(error, "23514", "credentials_issuer_scope");
}
#[tokio::test]
async fn cross_workspace_reference_rejected() {
    let db = Database::migrated().await;
    let a = fixture(&db.pool).await;
    let b = fixture(&db.pool).await;
    assert_db(
        sqlx::query("UPDATE handles SET inode_id=$1 WHERE workspace_id=$2")
            .bind(b.file)
            .bind(a.workspace)
            .execute(&db.pool)
            .await
            .unwrap_err(),
        "23503",
        "handles_inode_fk",
    );
    assert_db(
        sqlx::query("UPDATE read_pins SET manifest_id=$1 WHERE workspace_id=$2")
            .bind(b.manifest)
            .bind(a.workspace)
            .execute(&db.pool)
            .await
            .unwrap_err(),
        "23503",
        "read_pins_manifest_fk",
    );
    // Current manifest must belong to the same inode as well as the same workspace.
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("UPDATE inodes SET current_manifest_id=$1 WHERE workspace_id=$2 AND id=$3")
        .bind(b.manifest)
        .bind(a.workspace)
        .bind(a.file)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_db(
        tx.commit().await.unwrap_err(),
        "23503",
        "inodes_current_manifest_fk",
    );
}
#[tokio::test]
async fn manifests_immutable() {
    let db = Database::migrated().await;
    fixture(&db.pool).await;
    assert_db(
        sqlx::query("UPDATE manifests SET size=size+1")
            .execute(&db.pool)
            .await
            .unwrap_err(),
        "AF001",
        "manifests_immutable",
    );
    assert_db(
        sqlx::query("UPDATE manifest_chunks SET logical_length=1")
            .execute(&db.pool)
            .await
            .unwrap_err(),
        "AF001",
        "manifest_chunks_immutable",
    );
}
#[tokio::test]
async fn durability_settings_default_container_pass() {
    let db = Database::start(&[]).await;
    DurabilitySettings::read(&db.pool)
        .await
        .unwrap()
        .check(DurabilityProfile::Durable)
        .unwrap();
    settings::check_encoding(&settings::read_server_encoding(&db.pool).await.unwrap()).unwrap();
    assert!(settings::check_encoding("LATIN1").is_err());
}
#[tokio::test]
async fn durability_settings_fsync_off_fails() {
    let db = Database::start(&["postgres", "-c", "fsync=off"]).await;
    let settings = DurabilitySettings::read(&db.pool).await.unwrap();
    assert_eq!(
        settings
            .check(DurabilityProfile::Durable)
            .unwrap_err()
            .setting,
        "fsync"
    );
    settings.check(DurabilityProfile::Development).unwrap();
    assert!(!settings.unsafe_settings().is_empty());
}
#[tokio::test]
async fn durability_settings_server_sync_commit_off_fails() {
    let db = Database::start(&["postgres", "-c", "synchronous_commit=off"]).await;
    let settings = DurabilitySettings::read(&db.pool).await.unwrap();
    assert_eq!(settings.synchronous_commit.setting, "on");
    assert_eq!(settings.synchronous_commit.reset_val, "off");
    let error = settings.check(DurabilityProfile::Durable).unwrap_err();
    assert_eq!(error.setting, "synchronous_commit");
    assert_eq!(error.source, "server");
}
#[tokio::test]
async fn durability_settings_role_override_detected() {
    let db = Database::start(&[]).await;
    sqlx::query("ALTER ROLE CURRENT_USER SET synchronous_commit=off")
        .execute(&db.pool)
        .await
        .unwrap();
    db.pool.close().await;
    let pool = pool::connect(&db.config).await.unwrap();
    let settings = DurabilitySettings::read(&pool).await.unwrap();
    assert_eq!(settings.synchronous_commit.setting, "on");
    let error = settings.check(DurabilityProfile::Durable).unwrap_err();
    assert_eq!(error.setting, "synchronous_commit");
    assert_eq!(error.source, "role");
}
#[tokio::test]
async fn receipt_key_unique() {
    let db = Database::migrated().await;
    fixture(&db.pool).await;
    assert_db(sqlx::query("INSERT INTO operations(workspace_id,id,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,result,created_at,committed_at,expires_at) SELECT workspace_id,$1,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,result,created_at,committed_at,expires_at FROM operations").bind(Uuid::new_v4()).execute(&db.pool).await.unwrap_err(),"23505","operations_receipt_key");
}
#[tokio::test]
async fn database_newer_detected() {
    let db = Database::migrated().await;
    assert!(!migrate::database_is_newer(&db.pool).await.unwrap());
    sqlx::query("INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) VALUES (9999,'synthetic future',true,decode('aa','hex'),0)").execute(&db.pool).await.unwrap();
    assert!(migrate::database_is_newer(&db.pool).await.unwrap());
}

#[tokio::test]
async fn credential_scope_cannot_transfer_during_worker_issuance() {
    let db = Database::migrated().await;
    let a = fixture(&db.pool).await;
    let b = fixture(&db.pool).await;
    let issuer: Uuid =
        sqlx::query_scalar("SELECT issued_by_credential_id FROM credentials WHERE id=$1")
            .bind(a.host)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let mut transfer = db.pool.begin().await.unwrap();
    // Reproduce the two-connection schedule: the issuer's UPDATE remains in flight while
    // a child is issued in another workspace. The transfer must fail before either can commit.
    let error =
        sqlx::query("UPDATE credentials SET kind='host',workspace_id=$2,role='write' WHERE id=$1")
            .bind(issuer)
            .bind(a.workspace)
            .execute(&mut *transfer)
            .await
            .unwrap_err();
    assert_db(error, "23514", "credentials_identity_immutable");
    let mut issuance = db.pool.begin().await.unwrap();
    sqlx::query("INSERT INTO credentials(id,workspace_id,kind,role,issued_by_credential_id,uid,gid,secret_hash,max_reservation_bytes,created_at) VALUES($1,$2,'worker','write',$3,1000,1000,decode(repeat('ab',32),'hex'),4096,now())").bind(Uuid::new_v4()).bind(b.workspace).bind(issuer).execute(&mut *issuance).await.unwrap();
    issuance.commit().await.unwrap();
    transfer.rollback().await.unwrap();
    let invalid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM credentials child JOIN credentials issuer ON issuer.id=child.issued_by_credential_id WHERE issuer.workspace_id IS NOT NULL AND child.workspace_id IS DISTINCT FROM issuer.workspace_id)").fetch_one(&db.pool).await.unwrap();
    assert!(!invalid);
}

#[tokio::test]
async fn expired_receipts_retain_identity_and_referenced_state() {
    let db = Database::migrated().await;
    let f = fixture(&db.pool).await;
    let mut tx = db.pool.begin().await.unwrap();
    // Structural demonstration of conceptual receipt_expire, not a production expiry worker.
    sqlx::query("UPDATE operations SET expires_at=now()-interval '1 second',state='indeterminate',result=NULL WHERE workspace_id=$1").bind(f.workspace).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE admin_operations SET expires_at=now()-interval '1 second',state='indeterminate',result=NULL").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "UPDATE client_sessions SET revoked_at=now(),session_epoch=session_epoch+1 WHERE id=$1",
    )
    .bind(f.session)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let retained:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM inodes i JOIN manifests m ON (m.workspace_id,m.id)=(i.workspace_id,i.current_manifest_id) JOIN operations o ON (o.workspace_id,o.id)=(m.workspace_id,m.created_by_operation_id) JOIN upload_reservations r ON (r.workspace_id,r.operation_id)=(o.workspace_id,o.id) JOIN objects b ON (b.workspace_id,b.reservation_id)=(r.workspace_id,r.id) WHERE i.workspace_id=$1 AND o.state='indeterminate' AND o.result IS NULL)").bind(f.workspace).fetch_one(&db.pool).await.unwrap();
    assert!(retained);
    assert_db(sqlx::query("INSERT INTO operations(workspace_id,id,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,result,created_at,committed_at,expires_at) SELECT workspace_id,$2,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,result,created_at,committed_at,expires_at FROM operations WHERE workspace_id=$1").bind(f.workspace).bind(Uuid::new_v4()).execute(&db.pool).await.unwrap_err(),"23505","operations_receipt_key");
    let admin_key: Option<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM admin_operations WHERE state='indeterminate' LIMIT 1",
    )
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert!(admin_key.is_some());
    assert_db(sqlx::query("INSERT INTO admin_operations SELECT $1,credential_id,credential_epoch_at_start,idempotency_key,op_type,request_digest,owner_token,heartbeat_at,state,result,created_at,committed_at,expires_at FROM admin_operations LIMIT 1").bind(Uuid::new_v4()).execute(&db.pool).await.unwrap_err(),"23505","admin_operations_credential_id_idempotency_key_key");
}

#[tokio::test]
async fn authority_load_rejects_wrong_secret_revoked_and_expired() {
    use anchorfs_core::{CredentialId, CredentialProof, random_token, secret_hash};
    use anchorfs_persistence::authority::{self, AuthorityError, LockMode};
    let db = Database::migrated().await;
    let id = CredentialId::new();
    let secret = random_token();
    sqlx::query("INSERT INTO credentials (id,kind,role,uid,gid,secret_hash,max_reservation_bytes,created_at) VALUES ($1,'administrator','admin',1,2,$2,4096,now())")
        .bind(id.0).bind(secret_hash(secret.expose()).0.as_slice()).execute(&db.pool).await.unwrap();
    let proof = CredentialProof { id, secret };
    let mut tx = db.pool.begin().await.unwrap();
    assert_eq!(
        authority::load_actor(&mut tx, &proof, LockMode::Share)
            .await
            .unwrap()
            .uid,
        1
    );
    let wrong = CredentialProof {
        id,
        secret: random_token(),
    };
    assert!(matches!(
        authority::load_actor(&mut tx, &wrong, LockMode::Share).await,
        Err(AuthorityError::Unauthenticated)
    ));
    tx.rollback().await.unwrap();
    for assignment in [
        "expires_at=clock_timestamp()-interval '1 second'",
        "expires_at=NULL,revoked_at=clock_timestamp()",
    ] {
        // Fixed synthetic assignments; no request-derived SQL fragments.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE credentials SET {assignment} WHERE id=$1"
        )))
        .bind(id.0)
        .execute(&db.pool)
        .await
        .unwrap();
        let mut tx = db.pool.begin().await.unwrap();
        assert!(matches!(
            authority::load_actor(&mut tx, &proof, LockMode::Share).await,
            Err(AuthorityError::Unauthenticated)
        ));
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn receipt_begin_replays_mismatches_and_tombstones() {
    use anchorfs_core::CredentialId;
    use anchorfs_persistence::receipts::{
        self, ReceiptEntry, ReceiptFilter, ReceiptRequest, ReceiptScope,
    };
    let db = Database::migrated().await;
    let fixture = fixture(&db.pool).await;
    let scope = ReceiptScope::Admin {
        credential: CredentialId(fixture.host),
    };
    let request = |digest| ReceiptRequest {
        key: "new-key",
        op_type: "session_create",
        digest,
        credential_epoch: 1,
        retention: Duration::from_secs(3600),
        wait_bound: Duration::from_secs(1),
    };
    let mut tx = db.pool.begin().await.unwrap();
    let ReceiptEntry::Fresh {
        operation_id: id, ..
    } = receipts::begin(&mut tx, scope, request([1; 32]))
        .await
        .unwrap()
    else {
        panic!("fresh receipt expected");
    };
    let result = serde_json::json!({"published": true});
    receipts::commit_result(&mut tx, scope, id, &result)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        matches!(receipts::begin(&mut tx, scope, request([1;32])).await.unwrap(), ReceiptEntry::Replay { operation_id, result: r } if operation_id == id && r == result)
    );
    assert!(
        matches!(receipts::begin(&mut tx, scope, request([2;32])).await.unwrap(), ReceiptEntry::Mismatch { operation_id } if operation_id == id)
    );
    tx.commit().await.unwrap();
    sqlx::query(
        "UPDATE admin_operations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(id.0)
    .execute(&db.pool)
    .await
    .unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let filter = ReceiptFilter {
        workspace: None,
        credential: CredentialId(fixture.host),
        allow_workspace_actors: false,
    };
    let view = receipts::read_by_key(&mut tx, filter, "new-key")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.state, "indeterminate");
    assert_eq!(view.result, None);
    tx.commit().await.unwrap();
    let persisted: (String, Option<serde_json::Value>, Uuid, String) = sqlx::query_as(
        "SELECT state,result,credential_id,idempotency_key FROM admin_operations WHERE id=$1",
    )
    .bind(id.0)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        persisted,
        ("indeterminate".into(), None, fixture.host, "new-key".into())
    );
    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        matches!(receipts::begin(&mut tx, scope, request([1;32])).await.unwrap(), ReceiptEntry::Expired { operation_id, .. } if operation_id == id)
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn receipt_duplicate_insert_waits_for_owner() {
    use anchorfs_core::CredentialId;
    use anchorfs_persistence::receipts::{
        self, ReceiptEntry, ReceiptError, ReceiptRequest, ReceiptScope,
    };
    let db = Database::migrated().await;
    let fixture = fixture(&db.pool).await;
    let scope = ReceiptScope::Admin {
        credential: CredentialId(fixture.host),
    };
    for outcome in ["commit", "rollback", "timeout"] {
        let request = |wait_bound| ReceiptRequest {
            key: outcome,
            op_type: "session_create",
            digest: [3; 32],
            credential_epoch: 1,
            retention: Duration::from_secs(3600),
            wait_bound,
        };
        let mut owner = db.pool.begin().await.unwrap();
        let ReceiptEntry::Fresh {
            operation_id: id, ..
        } = receipts::begin(&mut owner, scope, request(Duration::from_secs(5)))
            .await
            .unwrap()
        else {
            panic!("fresh receipt expected");
        };
        receipts::commit_result(&mut owner, scope, id, &serde_json::json!({"ok":true}))
            .await
            .unwrap();
        let mut waiter = db.pool.begin().await.unwrap();
        if outcome == "timeout" {
            let start = std::time::Instant::now();
            assert!(matches!(
                receipts::begin(&mut waiter, scope, request(Duration::from_millis(200))).await,
                Err(ReceiptError::PendingOwner)
            ));
            assert!(start.elapsed() < Duration::from_millis(1200));
            let error = sqlx::query("SELECT 1")
                .execute(&mut *waiter)
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("25P02")
            );
            waiter.rollback().await.unwrap();
            owner.rollback().await.unwrap();
            continue;
        }
        let future = receipts::begin(&mut waiter, scope, request(Duration::from_secs(5)));
        tokio::pin!(future);
        assert!(
            tokio::time::timeout(Duration::from_millis(500), &mut future)
                .await
                .is_err()
        );
        if outcome == "commit" {
            owner.commit().await.unwrap();
        } else {
            owner.rollback().await.unwrap();
        }
        let entry = future.await.unwrap();
        if outcome == "commit" {
            assert!(
                matches!(entry, ReceiptEntry::Replay { operation_id, .. } if operation_id == id)
            );
        } else {
            assert!(matches!(
                entry,
                ReceiptEntry::Fresh {
                    operation_id: _,
                    ..
                }
            ));
        }
        // waiter rolls back when dropped, after the borrowed future is dropped.
    }
}
