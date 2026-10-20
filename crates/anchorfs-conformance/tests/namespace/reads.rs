use crate::common::fixture::*;
#[tokio::test]
async fn namespace_reads_page_in_byte_order_and_bind_cursors() {
    let f = Fixture::new().await;
    let prefix = f.workspace_path();
    for (i, name) in ["B", "a", "é", "_", "Z"].iter().enumerate() {
        f.client
            .call(
                "POST",
                &format!("{prefix}/directories"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(&format!("mkdir{i}")),
                Some(json!({"path":name,"mode":493})),
            )
            .await
            .status(200);
    }
    let stat = f
        .client
        .call(
            "GET",
            &format!("{prefix}/stat?path=B"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(stat["kind"], "directory");
    assert_eq!(stat["nlink"], 2);
    let mut names = vec![];
    let mut cursor = None;
    loop {
        let url = format!(
            "{prefix}/directory?limit=2{}",
            cursor
                .as_ref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let page = f
            .client
            .call(
                "GET",
                &url,
                Some(&f.worker),
                Some(&f.worker_session),
                None,
                None,
            )
            .await
            .status(200)
            .body;
        let _: anchorfs_service::dto::DirectoryPage = serde_json::from_value(page.clone()).unwrap();
        names.extend(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["name"].as_str().unwrap().to_owned()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if let Some(cursor) = &cursor {
            let error = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/directory?path=B&cursor={cursor}"),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    None,
                    None,
                )
                .await
                .error(400, "invalid_input");
            assert_eq!(error.body["message"], "cursor_scope");
            let mut bad = cursor.clone();
            bad.pop();
            bad.push('g');
            let error = f
                .client
                .call(
                    "GET",
                    &format!("{prefix}/directory?cursor={bad}"),
                    Some(&f.worker),
                    Some(&f.worker_session),
                    None,
                    None,
                )
                .await
                .error(400, "invalid_input");
            assert_eq!(error.body["message"], "cursor_invalid");
        } else {
            break;
        }
    }
    assert_eq!(names, ["B", "Z", "_", "a", "é"]);
    f.client
        .call(
            "POST",
            &format!("{prefix}/symlinks"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("sym"),
            Some(json!({"path":"sym","target":"/opaque"})),
        )
        .await
        .status(200);
    let read = f
        .client
        .call(
            "GET",
            &format!("{prefix}/symlinks?path=sym"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_eq!(read["target"], "/opaque");
    f.client
        .call(
            "GET",
            &format!("{prefix}/stat?path=sym/child"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(409, "not_directory");
    assert!(
        anchorfs_persistence::invariants::check(&f.pool, anchorfs_core::WorkspaceId(f.workspace))
            .await
            .unwrap()
            .is_empty()
    );
    f.backend.untouched();
}
#[tokio::test]
async fn ns8_cursor_expiry_workspace_and_key_scope() {
    let f = Fixture::with(workspace_body(), |c| c.cursor_ttl = Duration::from_secs(1)).await;
    let prefix = f.workspace_path();
    for name in ["a", "b"] {
        f.client
            .call(
                "POST",
                &format!("{prefix}/directories"),
                Some(&f.worker),
                Some(&f.worker_session),
                Some(name),
                Some(json!({"path":name,"mode":493})),
            )
            .await
            .status(200);
    }
    let page = f
        .client
        .call(
            "GET",
            &format!("{prefix}/directory?limit=1"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    let cursor = page["next_cursor"].as_str().unwrap();
    let url = format!("{prefix}/directory?cursor={cursor}");
    f.client
        .call(
            "GET",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200);
    let other = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("other"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let w = other["workspace"]["workspace_id"].as_str().unwrap();
    let host=Credential::from_grant(&f.client.call("POST","/v1/credentials",Some(&f.admin),None,Some("other-host"),Some(json!({"workspace_id":w,"kind":"host","role":"write","uid":1000,"gid":1000,"max_reservation_bytes":4096}))).await.status(201).body);
    let session = f
        .client
        .call(
            "POST",
            &format!("/v1/workspaces/{w}/sessions"),
            Some(&f.admin),
            None,
            Some("other-session"),
            Some(json!({"credential_id":host.id})),
        )
        .await
        .status(201)
        .body;
    let r = f
        .client
        .call(
            "GET",
            &format!("/v1/workspaces/{w}/directory?cursor={cursor}"),
            Some(&host),
            Some(&session),
            None,
            None,
        )
        .await
        .error(400, "invalid_input");
    assert_eq!(r.body["message"], "cursor_scope");
    let mut config = Config::contract_defaults();
    config.cursor_signing_key = anchorfs_core::Secret::new("ff".repeat(32));
    let client = Client {
        app: build_router(AppState {
            cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose()).unwrap(),
            pool: f.pool.clone(),
            config: Arc::new(config),
            backend: f.backend.service(),
            probe_cache: Default::default(),
            data_permits: Arc::new(tokio::sync::Semaphore::new(6)),
            faults: Arc::new(anchorfs_service::pipeline::faults::NoFaults),
            control_permits: Arc::new(tokio::sync::Semaphore::new(2)),
        }),
    };
    let r = client
        .call(
            "GET",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(400, "invalid_input");
    assert_eq!(r.body["message"], "cursor_invalid");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let r = f
        .client
        .call(
            "GET",
            &url,
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .error(400, "invalid_input");
    assert_eq!(r.body["message"], "cursor_expired");
    f.invariants().await;
}
#[tokio::test]
async fn ns9_large_listing_and_generation_change() {
    let mut body = workspace_body();
    body["logical_inodes_limit"] = json!(3000);
    let f = Fixture::with(body, |_| {}).await;
    let prefix = f.workspace_path();
    let mut tx = f.pool.begin().await.unwrap();
    // Synthetic sequential names and empty file inodes, in the disposable fixture only.
    sqlx::query("WITH new AS (INSERT INTO inodes(workspace_id,kind,mode,uid,gid,atime,mtime,ctime) SELECT $1,'file',420,1000,1000,now(),now(),now() FROM generate_series(1,2501) RETURNING id), numbered AS (SELECT id,row_number() OVER (ORDER BY id) AS n FROM new) INSERT INTO directory_entries(workspace_id,parent_inode_id,name,inode_id) SELECT $1,w.root_inode_id,'n'||lpad(n.n::text,6,'0'),n.id FROM numbered n JOIN workspaces w ON w.id=$1").bind(f.workspace).execute(&mut *tx).await.unwrap();
    let operation = Uuid::new_v4();
    sqlx::query("INSERT INTO operations(workspace_id,id,credential_id,client_session_id,session_epoch,credential_epoch_at_start,authorization_epoch_at_start,mutation_epoch_at_start,owner_token,heartbeat_at,idempotency_key,op_type,request_digest,state,result,created_at,committed_at,expires_at) SELECT s.workspace_id,$2,s.credential_id,s.id,s.session_epoch,s.credential_epoch,w.authorization_epoch,w.mutation_epoch,decode(repeat('12',32),'hex'),clock_timestamp(),'synthetic-listing-seed','open',decode(repeat('34',32),'hex'),'committed','{}'::jsonb,transaction_timestamp(),transaction_timestamp(),clock_timestamp()+interval '1 day' FROM client_sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.workspace_id=$1 AND s.id=$3")
        .bind(f.workspace).bind(operation).bind(Uuid::parse_str(f.worker_session["session_id"].as_str().unwrap()).unwrap()).execute(&mut *tx).await.unwrap();
    sqlx::query("WITH descriptions AS (INSERT INTO manifests(workspace_id,id,inode_id,size,chunk_size,generation,created_by_operation_id,created_at) SELECT workspace_id,gen_random_uuid(),id,0,4194304,generation,$2,transaction_timestamp() FROM inodes WHERE workspace_id=$1 AND kind='file' AND current_manifest_id IS NULL RETURNING inode_id,id) UPDATE inodes SET current_manifest_id=d.id FROM descriptions d WHERE inodes.workspace_id=$1 AND inodes.id=d.inode_id")
        .bind(f.workspace).bind(operation).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE workspace_quotas SET logical_inodes_used=2501 WHERE workspace_id=$1")
        .bind(f.workspace)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    f.invariants().await;
    let mut names = vec![];
    let mut next = None;
    let mut sizes = vec![];
    loop {
        let url = format!(
            "{prefix}/directory?limit=1000{}",
            next.as_ref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let page = f
            .client
            .call(
                "GET",
                &url,
                Some(&f.worker),
                Some(&f.worker_session),
                None,
                None,
            )
            .await
            .status(200)
            .body;
        sizes.push(page["entries"].as_array().unwrap().len());
        names.extend(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["name"].as_str().unwrap().to_owned()),
        );
        next = page["next_cursor"].as_str().map(str::to_owned);
        if next.is_none() {
            break;
        }
    }
    assert_eq!(sizes, [1000, 1000, 501]);
    assert_eq!(
        names,
        (1..=2501).map(|i| format!("n{i:06}")).collect::<Vec<_>>()
    );
    let first = f
        .client
        .call(
            "GET",
            &format!("{prefix}/directory?limit=1"),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    f.client
        .call(
            "POST",
            &format!("{prefix}/directories"),
            Some(&f.worker),
            Some(&f.worker_session),
            Some("change"),
            Some(json!({"path":"z","mode":493})),
        )
        .await
        .status(200);
    let second = f
        .client
        .call(
            "GET",
            &format!(
                "{prefix}/directory?cursor={}",
                first["next_cursor"].as_str().unwrap()
            ),
            Some(&f.worker),
            Some(&f.worker_session),
            None,
            None,
        )
        .await
        .status(200)
        .body;
    assert_ne!(
        first["namespace_generation"],
        second["namespace_generation"]
    );
    f.invariants().await;
}
