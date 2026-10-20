//! Bounded session-owned references to immutable manifests.
use crate::{
    authority::ActorSession,
    manifests::{self, ManifestRow},
};
use anchorfs_core::{ManifestId, ReadPinId, SessionId, WorkspaceId};
use sqlx::{Postgres, Transaction};
use std::time::Duration;
use time::OffsetDateTime;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PinRow {
    pub workspace_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub manifest_id: uuid::Uuid,
    pub client_session_id: uuid::Uuid,
    pub expires_at: OffsetDateTime,
}
#[derive(Debug, thiserror::Error)]
pub enum PinError {
    #[error("read pin limit reached")]
    Limit,
    #[error("read pin expired")]
    Stale,
    #[error("read pin not found")]
    NotFound,
    #[error("read pin belongs to another inode")]
    Mismatch,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
pub async fn lock_admission(
    tx: &mut Transaction<'_, Postgres>,
    session: SessionId,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,7003))")
        .bind(session.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn count_live(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: SessionId,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM read_pins WHERE workspace_id=$1 AND client_session_id=$2 AND expires_at>clock_timestamp()")
        .bind(workspace.0).bind(session.0).fetch_one(&mut **tx).await
}
pub async fn create(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    manifest: ManifestId,
    ttl: Duration,
    limit: u32,
) -> Result<PinRow, PinError> {
    if session.workspace_id != workspace {
        return Err(PinError::NotFound);
    }
    lock_admission(tx, session.id).await?;
    if count_live(tx, workspace, session.id).await? >= i64::from(limit) {
        return Err(PinError::Limit);
    }
    Ok(sqlx::query_as("INSERT INTO read_pins(workspace_id,id,manifest_id,client_session_id,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+$5 * interval '1 second') RETURNING *")
        .bind(workspace.0).bind(ReadPinId::new().0).bind(manifest.0).bind(session.id.0)
        .bind(ttl.as_secs_f64()).fetch_one(&mut **tx).await?)
}
async fn load(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: ReadPinId,
    update: bool,
) -> Result<PinRow, PinError> {
    if session.workspace_id != workspace {
        return Err(PinError::NotFound);
    }
    let query = if update {
        "SELECT * FROM read_pins WHERE workspace_id=$1 AND id=$2 AND client_session_id=$3 FOR UPDATE"
    } else {
        "SELECT * FROM read_pins WHERE workspace_id=$1 AND id=$2 AND client_session_id=$3 FOR SHARE"
    };
    let row: PinRow = sqlx::query_as(query)
        .bind(workspace.0)
        .bind(id.0)
        .bind(session.id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(PinError::NotFound)?;
    let expired: bool = sqlx::query_scalar("SELECT $1::timestamptz <= clock_timestamp()")
        .bind(row.expires_at)
        .fetch_one(&mut **tx)
        .await?;
    if expired {
        return Err(PinError::Stale);
    }
    Ok(row)
}
pub async fn load_for_read(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: ReadPinId,
    inode: i64,
) -> Result<(PinRow, ManifestRow), PinError> {
    let pin = load(tx, workspace, session, id, false).await?;
    let manifest = manifests::load(tx, workspace, ManifestId(pin.manifest_id))
        .await
        .map_err(|error| match error {
            manifests::ManifestError::Database(error) => PinError::Database(error),
            _ => PinError::NotFound,
        })?;
    if manifest.inode_id != inode {
        return Err(PinError::Mismatch);
    }
    Ok((pin, manifest))
}
pub async fn release(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    session: &ActorSession,
    id: ReadPinId,
) -> Result<PinRow, PinError> {
    load(tx, workspace, session, id, true).await?;
    Ok(sqlx::query_as("UPDATE read_pins SET expires_at=clock_timestamp() WHERE workspace_id=$1 AND id=$2 RETURNING *")
        .bind(workspace.0).bind(id.0).fetch_one(&mut **tx).await?)
}
