use super::{Client, MountConfig, MountError};
use anchorfs_client::{auth::SessionAuthority, error::ClientError, retry::IdempotencyKey};
use anchorfs_core::{ErrorCode, SessionId};
use std::time::Duration;

pub(super) struct Established {
    pub client: Client,
    pub self_minted: bool,
    pub root: i64,
}

pub(super) async fn connect(cfg: &MountConfig) -> Result<Established, MountError> {
    let mut client = Client::new(
        cfg.endpoint.clone(),
        cfg.workspace,
        cfg.authority.clone(),
        cfg.ca_bundle.as_deref(),
    )
    .map_err(|e| MountError::Config(e.to_string()))?
    .with_deadlines(cfg.metadata_deadline, cfg.data_deadline);
    let self_minted = client.authority().session.is_none();
    if self_minted {
        let grant = client
            .create_session(
                client.authority().bearer.credential_id.0,
                &IdempotencyKey::new(),
            )
            .await
            .map_err(|e| {
                if e.kind() == Some(ErrorCode::Forbidden) {
                    MountError::Authorization(
                        "worker credentials require a host-minted session".into(),
                    )
                } else {
                    startup_error(e)
                }
            })?;
        let mut authority = client.authority();
        authority.session = Some(SessionAuthority {
            session_id: SessionId(grant.session_id),
            session_epoch: grant.session_epoch,
        });
        client = client.with_authority(authority);
    }
    match validate(&client, cfg).await {
        Ok(root) => Ok(Established {
            client,
            self_minted,
            root,
        }),
        Err(error) => {
            if self_minted {
                let session = client.authority().session.expect("session established");
                let _ = tokio::time::timeout(
                    Duration::from_secs(10),
                    client.revoke_session(session.session_id.0, &IdempotencyKey::new()),
                )
                .await;
            }
            Err(error)
        }
    }
}

async fn validate(client: &Client, cfg: &MountConfig) -> Result<i64, MountError> {
    let workspace = client.get_workspace().await.map_err(startup_error)?;
    let root = client.stat(None).await.map_err(startup_error)?;
    if root.inode_id != workspace.root.inode_id {
        return Err(MountError::Authorization(
            "workspace root changed during startup".into(),
        ));
    }
    if let Some(proof) = client.authority().workflow
        && !workspace
            .workflow
            .as_ref()
            .is_some_and(|live| live.active && live.mutation_epoch == proof.epoch)
    {
        return Err(MountError::Authorization(
            "workflow epoch does not match a live lease".into(),
        ));
    }
    let session = client.authority().session.expect("session established");
    let renewed = client
        .renew_session(session.session_id.0, &IdempotencyKey::new())
        .await
        .map_err(startup_error)?;
    tracing::info!(expires_at = %renewed.expires_at, "mount session renewed at startup");
    if !cfg.allow_other {
        match effective_ids() {
            Some((uid, gid)) if uid != root.uid || gid != root.gid => tracing::warn!(
                "mount uid/gid differ from workspace root; default_permissions can reject the caller"
            ),
            None => tracing::warn!(
                "effective uid/gid unavailable; cannot check workspace identity alignment"
            ),
            _ => {}
        }
    }
    Ok(root.inode_id)
}

fn startup_error(error: ClientError) -> MountError {
    if matches!(
        error.kind(),
        Some(
            ErrorCode::Unauthenticated
                | ErrorCode::Forbidden
                | ErrorCode::NotFound
                | ErrorCode::Fenced
        )
    ) || matches!(&error, ClientError::Api(e) if matches!(e.status, 401 | 403 | 404))
    {
        MountError::Authorization(error.to_string())
    } else {
        MountError::Unavailable(error.to_string())
    }
}

// Linux provides real/effective/saved/filesystem ids in this documented order.
fn effective_ids() -> Option<(u32, u32)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let id = |field| {
        status
            .lines()
            .find(|line| line.starts_with(field))?
            .split_whitespace()
            .nth(2)?
            .parse()
            .ok()
    };
    Some((id("Uid:")?, id("Gid:")?))
}
