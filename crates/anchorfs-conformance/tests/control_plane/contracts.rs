use super::support::*;
use anchorfs_service::dto::{
    CreatedWorkspace, InodeKindDto, IssuedCredential, MutationResult, OperationStateDto,
    OperationStatus, SessionGrant, WorkspaceInfo,
};

#[tokio::test]
async fn runtime_success_and_receipt_bodies_match_declared_dtos() {
    let f = Fixture::new().await;
    let created = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("typed-workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let created: CreatedWorkspace = serde_json::from_value(created).unwrap();
    assert!(matches!(
        created.workspace.root.kind,
        InodeKindDto::Directory
    ));
    let replay = f
        .client
        .call(
            "POST",
            "/v1/workspaces",
            Some(&f.admin),
            None,
            Some("typed-workspace"),
            Some(workspace_body()),
        )
        .await
        .status(201)
        .body;
    let replay: CreatedWorkspace = serde_json::from_value(replay).unwrap();
    assert_eq!(
        created.workspace.workspace_id,
        replay.workspace.workspace_id
    );
    assert_eq!(created.operation_id, replay.operation_id);
    let workspace: WorkspaceInfo = serde_json::from_value(
        f.client
            .call("GET", &f.workspace_path(), Some(&f.host), None, None, None)
            .await
            .status(200)
            .body,
    )
    .unwrap();
    assert_eq!(workspace.workspace_id, f.workspace);
    let fresh = f
        .client
        .call(
            "POST",
            "/v1/credentials",
            Some(&f.admin),
            None,
            Some("typed-worker"),
            Some(credential_body(f.workspace, "worker")),
        )
        .await
        .status(201);
    assert_eq!(fresh.headers["Cache-Control"], "no-store");
    let grant: IssuedCredential = serde_json::from_value(fresh.body).unwrap();
    assert!(grant.secret.is_some());
    let replay: IssuedCredential = serde_json::from_value(
        f.client
            .call(
                "POST",
                "/v1/credentials",
                Some(&f.admin),
                None,
                Some("typed-worker"),
                Some(credential_body(f.workspace, "worker")),
            )
            .await
            .status(201)
            .body,
    )
    .unwrap();
    assert_eq!(replay.credential_id, grant.credential_id);
    assert!(replay.secret.is_none());
    let status: OperationStatus = serde_json::from_value(
        f.client
            .call(
                "GET",
                "/v1/admin-operations?idempotency_key=typed-worker",
                Some(&f.admin),
                None,
                None,
                None,
            )
            .await
            .status(200)
            .body,
    )
    .unwrap();
    assert!(matches!(status.state, OperationStateDto::Committed));
    let stored: IssuedCredential = serde_json::from_value(status.result.unwrap()).unwrap();
    assert!(stored.secret.is_none());
    let rotated: IssuedCredential = serde_json::from_value(
        f.client
            .call(
                "POST",
                &format!("/v1/credentials/{}/rotate", grant.credential_id),
                Some(&f.admin),
                None,
                Some("typed-rotate"),
                None,
            )
            .await
            .status(200)
            .body,
    )
    .unwrap();
    assert_eq!(rotated.credential_epoch, 2);
    let mint = format!("{}/sessions", f.workspace_path());
    let session: SessionGrant = serde_json::from_value(
        f.client
            .call(
                "POST",
                &mint,
                Some(&f.host),
                None,
                Some("typed-session"),
                Some(json!({"credential_id":f.worker.id})),
            )
            .await
            .status(201)
            .body,
    )
    .unwrap();
    assert_eq!(session.credential_epoch, 1);
    let renewed: SessionGrant = serde_json::from_value(
        f.client
            .call(
                "POST",
                &f.session_path(&f.worker_session, "/renew"),
                Some(&f.host),
                Some(&f.host_session),
                Some("typed-renew"),
                None,
            )
            .await
            .status(200)
            .body,
    )
    .unwrap();
    assert_eq!(renewed.session_epoch, 1);
    let revoked: MutationResult = serde_json::from_value(
        f.client
            .call(
                "DELETE",
                &f.session_path(&f.worker_session, ""),
                Some(&f.host),
                Some(&f.host_session),
                Some("typed-revoke"),
                None,
            )
            .await
            .status(200)
            .body,
    )
    .unwrap();
    assert!(revoked.inode_id.is_none());
    assert!(revoked.generation.is_none());
}
