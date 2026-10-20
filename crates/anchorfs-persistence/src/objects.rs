//! Object bookkeeping precedes backend I/O. Failed/uncertain attempted uploads
//! retain physical charges until a future offline collector confirms deletion.
use crate::reservations::{self, ReservationError};
use anchorfs_core::{ObjectId, OperationId, ReservationId, Sha256Digest, WorkspaceId};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ObjectRow {
    pub workspace_id: uuid::Uuid,
    pub id: uuid::Uuid,
    pub byte_length: i64,
    pub sha256: Vec<u8>,
    pub state: String,
    pub reservation_id: uuid::Uuid,
    pub uploaded_at: Option<OffsetDateTime>,
    pub published_at: Option<OffsetDateTime>,
}
pub async fn stage(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    reservation: ReservationId,
    operation: OperationId,
    id: ObjectId,
    length: u32,
    digest: Sha256Digest,
) -> Result<ObjectRow, ReservationError> {
    reservations::lock_active_share(tx, workspace, reservation, operation).await?;
    Ok(sqlx::query_as("INSERT INTO objects(workspace_id,id,byte_length,sha256,state,reservation_id) VALUES($1,$2,$3,$4,'staged',$5) RETURNING *")
        .bind(workspace.0).bind(id.0).bind(i64::from(length)).bind(digest.0.as_slice())
        .bind(reservation.0).fetch_one(&mut **tx).await?)
}
pub async fn mark_uploaded(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    ids: &[ObjectId],
) -> Result<(), sqlx::Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.0).collect();
    sqlx::query("UPDATE objects SET uploaded_at=clock_timestamp() WHERE workspace_id=$1 AND id=ANY($2) AND state='staged'")
        .bind(workspace.0).bind(ids).execute(&mut **tx).await?;
    Ok(())
}
pub async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    reservation: ReservationId,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("WITH updated AS (UPDATE objects SET state='published',published_at=transaction_timestamp() WHERE workspace_id=$1 AND reservation_id=$2 AND state='staged' RETURNING byte_length) SELECT COALESCE(sum(byte_length),0)::bigint FROM updated")
        .bind(workspace.0).bind(reservation.0).fetch_one(&mut **tx).await
}
pub async fn orphan(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    reservation: ReservationId,
    uncertain: &[ObjectId],
    skip: &[ObjectId],
    reason: &str,
) -> Result<i64, sqlx::Error> {
    let uncertain: Vec<_> = uncertain.iter().map(|id| id.0).collect();
    let skip: Vec<_> = skip.iter().map(|id| id.0).collect();
    sqlx::query("UPDATE objects SET state='uncertain' WHERE workspace_id=$1 AND reservation_id=$2 AND id=ANY($3) AND state='staged'")
        .bind(workspace.0).bind(reservation.0).bind(uncertain).execute(&mut **tx).await?;
    sqlx::query_scalar("WITH inserted AS (INSERT INTO deletion_candidates(workspace_id,object_id,reason,physical_bytes,recorded_at) SELECT workspace_id,id,$4,byte_length,clock_timestamp() FROM objects WHERE workspace_id=$1 AND reservation_id=$2 AND state<>'published' AND id<>ALL($3::uuid[]) ON CONFLICT DO NOTHING RETURNING physical_bytes) SELECT COALESCE(sum(physical_bytes),0)::bigint FROM inserted")
        .bind(workspace.0).bind(reservation.0).bind(skip).bind(reason).fetch_one(&mut **tx).await
}
