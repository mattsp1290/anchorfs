//! Receipt-first positional writes and append. Transaction ownership stays in
//! each phase; materialization and uploads receive no transaction or connection.
use super::{
    common::{self, Admission, AdmissionPlan, Context, Operation, Prepared, Upload},
    faults::{self, FaultInjector, FaultPoint},
    runner::{Db, Deadline, Outcome, PhaseCell, unavailable},
};
use crate::{
    app::AppState,
    dto::WriteResult,
    error::{self, ApiError},
    receipt::Answer,
};
use anchorfs_backend::key::ObjectKey;
use anchorfs_core::{
    ErrorCode, HandleId, ManifestId, ObjectId, ReservationId, Sha256Digest, WritePlan, materialize,
    plan_write, sha256,
};
use anchorfs_persistence::{
    handles,
    manifests::{self, ChunkRow, ManifestRow, NewChunk},
    namespace::{self, InodeRow},
    objects, quota,
    receipts::{self, ReceiptEntry},
    reservations,
};
use bytes::Bytes;

#[derive(Clone, Copy)]
pub enum WriteKind {
    At(u64),
    Append,
}
pub struct WriteRequest {
    pub context: Context,
    pub handle: HandleId,
    pub kind: WriteKind,
    pub body: Bytes,
}
struct Snapshot {
    inode: InodeRow,
    manifest: ManifestRow,
    rows: Vec<ChunkRow>,
    plan: WritePlan,
    offset: u64,
}
enum Observed {
    Snapshot(Box<Snapshot>),
    Receipt(ReceiptEntry),
}
enum Swap {
    Reserved(Upload),
    Failed(ApiError),
    Unknown(ApiError),
    Crashed,
}
enum Publication {
    Written(Answer),
    Conflict,
    Unknown(ApiError),
}
struct Writer<'a> {
    db: &'a Db,
    deadline: Deadline,
    faults: &'a dyn FaultInjector,
    state: &'a AppState,
    req: &'a WriteRequest,
}

impl Writer<'_> {
    async fn snapshot(&self, operation: Option<&Operation>) -> Result<Observed, ApiError> {
        let ctx = &self.req.context;
        let mut tx = self.db.begin().await?;
        let p = match operation {
            Some(operation) => ctx.resume(&mut tx, operation).await?,
            None => ctx.prologue(&mut tx).await?,
        };
        if operation.is_none()
            && let Some(entry) =
                receipts::peek(&mut tx, common::scope(&p), &ctx.key, ctx.digest).await?
        {
            ctx.check(&mut tx, &p).await?;
            tx.commit().await?;
            return Ok(Observed::Receipt(entry));
        }
        let handle =
            handles::load_for_write(&mut tx, ctx.workspace, &p.session, self.req.handle, None)
                .await?;
        let inode = namespace::load_inode(&mut tx, ctx.workspace, handle.inode_id).await?;
        let manifest = manifests::current(&mut tx, ctx.workspace, &inode).await?;
        if manifest.chunk_size as u32 != self.state.config.chunk_size {
            return Err(ApiError::new(ErrorCode::Unsupported, ""));
        }
        let offset = match self.req.kind {
            WriteKind::At(offset) => offset,
            WriteKind::Append => inode.size as u64,
        };
        let g = common::geometry(self.state);
        let preliminary = plan_write(
            g,
            inode.size as u64,
            offset,
            self.req.body.len() as u32,
            &[],
        )?;
        let indices: Vec<_> = preliminary.touched.iter().map(|t| t.index as i32).collect();
        let rows = manifests::chunks(
            &mut tx,
            ctx.workspace,
            ManifestId(manifest.id),
            Some(&indices),
        )
        .await?;
        let existing = rows
            .iter()
            .map(common::existing_chunk)
            .collect::<Result<Vec<_>, _>>()?;
        let plan = plan_write(
            g,
            inode.size as u64,
            offset,
            self.req.body.len() as u32,
            &existing,
        )?;
        quota::check_growth(&mut tx, ctx.workspace, plan.new_size as i64 - inode.size).await?;
        ctx.check(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(Observed::Snapshot(Box::new(Snapshot {
            inode,
            manifest,
            rows,
            plan,
            offset,
        })))
    }
    async fn prepare(&self, snapshot: &Snapshot) -> Result<Vec<Prepared>, ApiError> {
        let mut prepared = Vec::new();
        for touched in &snapshot.plan.touched {
            let old = if touched.needs_fetch {
                let row = snapshot
                    .rows
                    .iter()
                    .find(|r| r.chunk_index as u32 == touched.index)
                    .ok_or_else(unavailable)?;
                let digest = Sha256Digest(
                    row.sha256
                        .as_slice()
                        .try_into()
                        .map_err(|_| unavailable())?,
                );
                let key = ObjectKey::new(
                    &self.state.config.deployment_id,
                    self.req.context.workspace,
                    ObjectId(row.object_id),
                );
                Some(
                    self.deadline
                        .within(self.state.backend.get_verified(
                            &key,
                            digest,
                            row.object_length as u64,
                        ))
                        .await
                        .map_err(|_| unavailable())?
                        .map_err(error::backend)?,
                )
            } else {
                None
            };
            let bytes = Bytes::from(materialize(touched, old.as_deref(), &self.req.body));
            let digest = sha256(&bytes);
            prepared.push(Prepared {
                index: touched.index,
                object: ObjectId::new(),
                length: bytes.len() as u32,
                bytes,
                digest,
            });
        }
        Ok(prepared)
    }
    async fn swap(
        &self,
        operation: &Operation,
        previous: &Upload,
        chunks: Vec<Prepared>,
    ) -> Result<Swap, ApiError> {
        let ctx = &self.req.context;
        let mut tx = self.db.begin_final().await?;
        let p = ctx.resume(&mut tx, operation).await?;
        let op = receipts::lock_pending(&mut tx, operation.scope, operation.id, &operation.owner)
            .await?;
        if op.authorization_epoch_at_start != p.workspace.authorization_epoch
            || op.mutation_epoch_at_start != p.workspace.mutation_epoch
        {
            return Err(ApiError::new(ErrorCode::Fenced, ""));
        }
        handles::load_for_write(&mut tx, ctx.workspace, &p.session, self.req.handle, None).await?;
        reservations::lock_for_finalize(
            &mut tx,
            ctx.workspace,
            ReservationId(previous.reservation.id),
            operation.id,
        )
        .await?;
        sqlx::query("SAVEPOINT reservation_swap")
            .execute(&mut *tx)
            .await?;
        common::orphan(
            &mut tx,
            ctx.workspace,
            operation,
            previous,
            "generation_conflict",
        )
        .await?;
        let next = common::reserve(&mut tx, self.state, &p, operation.id, &chunks).await;
        let outcome = match next {
            Ok(upload) => Swap::Reserved(upload),
            Err(error) => {
                sqlx::query("ROLLBACK TO SAVEPOINT reservation_swap")
                    .execute(&mut *tx)
                    .await?;
                let mut error: ApiError = error.into();
                error.operation_id = Some(operation.id);
                if !faults::hit(self.faults, FaultPoint::BeforeFinalize, Some(operation.id)).await {
                    tx.rollback().await?;
                    return Ok(Swap::Crashed);
                }
                common::finalize_in(
                    &mut tx,
                    ctx.workspace,
                    operation,
                    previous,
                    common::failure_code(&error),
                )
                .await?;
                Swap::Failed(error)
            }
        };
        match tx.commit().await {
            Ok(()) => Ok(outcome),
            Err(error) if !common::commit_outcome_unknown(&error) => Err(error.into()),
            Err(_) => Ok(Swap::Unknown(common::pending_error(operation.id))),
        }
    }
    async fn upload(&self, upload: &mut Upload) -> Result<(), ApiError> {
        for chunk in &mut upload.chunks {
            if chunk.length == 0
                || chunk.bytes.len() != chunk.length as usize
                || chunk.length > self.state.config.chunk_size
            {
                return Err(unavailable());
            }
            let key = ObjectKey::new(
                &self.state.config.deployment_id,
                self.req.context.workspace,
                chunk.object,
            );
            upload.attempted += 1;
            self.deadline
                .within(self.state.backend.put(
                    &key,
                    std::mem::take(&mut chunk.bytes),
                    chunk.digest,
                ))
                .await
                .map_err(|_| unavailable())?
                .map_err(|_| unavailable())?;
            upload.confirmed += 1;
        }
        Ok(())
    }
    async fn publish(
        &self,
        operation: &Operation,
        upload: &Upload,
        snapshot: &Snapshot,
    ) -> Result<Publication, ApiError> {
        let ctx = &self.req.context;
        let mut tx = self.db.begin_final().await?;
        let p = ctx.resume(&mut tx, operation).await?;
        let op = receipts::lock_pending(&mut tx, operation.scope, operation.id, &operation.owner)
            .await?;
        if op.authorization_epoch_at_start != p.workspace.authorization_epoch
            || op.mutation_epoch_at_start != p.workspace.mutation_epoch
        {
            return Err(ApiError::new(ErrorCode::Fenced, ""));
        }
        let inode = namespace::lock_inodes(&mut tx, ctx.workspace, &[snapshot.inode.id])
            .await?
            .pop()
            .ok_or_else(unavailable)?;
        let handle =
            handles::load_for_write(&mut tx, ctx.workspace, &p.session, self.req.handle, None)
                .await?;
        if handle.inode_id != inode.id {
            return Err(unavailable());
        }
        if inode.generation != snapshot.inode.generation
            && inode.current_manifest_id != Some(snapshot.manifest.id)
        {
            tx.rollback().await?;
            return Ok(Publication::Conflict);
        }
        let reservation = reservations::lock_active(
            &mut tx,
            ctx.workspace,
            ReservationId(upload.reservation.id),
            operation.id,
        )
        .await?;
        let confirmed: Vec<_> = upload.chunks[..upload.confirmed]
            .iter()
            .map(|c| c.object)
            .collect();
        objects::mark_uploaded(&mut tx, ctx.workspace, &confirmed).await?;
        let published =
            objects::publish(&mut tx, ctx.workspace, ReservationId(reservation.id)).await?;
        if published != reservation.reserved_bytes {
            return Err(unavailable());
        }
        let chunks: Vec<_> = upload
            .chunks
            .iter()
            .map(|c| NewChunk {
                index: c.index,
                object: c.object,
                sha256: c.digest,
                logical_length: c.length,
            })
            .collect();
        let (_, next) = manifests::publish_write(
            &mut tx,
            ctx.workspace,
            &inode,
            operation.id,
            self.state.config.chunk_size,
            snapshot.plan.new_size,
            &chunks,
        )
        .await?;
        reservations::consume(&mut tx, ctx.workspace, &reservation).await?;
        quota::settle(
            &mut tx,
            ctx.workspace,
            reservation.reserved_bytes,
            published,
            next.size - inode.size,
        )
        .await?;
        let result = serde_json::to_value(WriteResult {
            operation_id: operation.id.0,
            generation: next.generation,
            bytes_written: self.req.body.len() as u64,
            offset: snapshot.offset,
        })
        .map_err(|_| unavailable())?;
        if !receipts::commit_result(&mut tx, operation.scope, operation.id, &result).await? {
            return Err(ApiError::new(ErrorCode::OperationFailed, ""));
        }
        ctx.check(&mut tx, &p).await?;
        handles::assert_live(&mut tx, &handle).await?;
        reservations::assert_unexpired(&mut tx, &reservation).await?;
        match tx.commit().await {
            Ok(()) => Ok(Publication::Written(Answer::metadata(result)?)),
            Err(error) if !common::commit_outcome_unknown(&error) => Err(error.into()),
            Err(_) => Ok(Publication::Unknown(common::pending_error(operation.id))),
        }
    }
}
pub async fn write(
    db: Db,
    deadline: Deadline,
    faults: std::sync::Arc<dyn FaultInjector>,
    phase: std::sync::Arc<PhaseCell>,
    state: AppState,
    req: WriteRequest,
) -> Result<Outcome<Answer>, ApiError> {
    let writer = Writer {
        db: &db,
        deadline,
        faults: faults.as_ref(),
        state: &state,
        req: &req,
    };
    let mut snapshot = match writer.snapshot(None).await? {
        Observed::Receipt(entry) => {
            return req.context.resolve(&db, entry).await.map(Outcome::Done);
        }
        Observed::Snapshot(snapshot) => snapshot,
    };
    if !faults::hit(faults.as_ref(), FaultPoint::AfterPeek, None).await
        || !faults::hit(faults.as_ref(), FaultPoint::BeforeGet, None).await
    {
        return Ok(Outcome::Crashed);
    }
    let prepared = writer.prepare(&snapshot).await?;
    if !faults::hit(faults.as_ref(), FaultPoint::BeforeReserve, None).await {
        return Ok(Outcome::Crashed);
    }
    if !phase.start_admission() {
        return Err(unavailable());
    }
    let (operation, mut upload) = match common::admit(
        &db,
        &req.context,
        &state,
        AdmissionPlan::Write {
            handle: req.handle,
            chunks: &prepared,
        },
    )
    .await?
    {
        Admission::Receipt(entry) => {
            return req.context.resolve(&db, entry).await.map(Outcome::Done);
        }
        Admission::Owned(operation, upload) => (operation, upload),
        Admission::Unknown(error) => return Err(error),
    };
    drop(prepared);
    phase.admitted(operation.id);
    for attempt in 0..state.config.max_retry_attempts {
        if !faults::hit(
            faults.as_ref(),
            FaultPoint::AfterReserve,
            Some(operation.id),
        )
        .await
            || !faults::hit(
                faults.as_ref(),
                FaultPoint::BeforeUpload,
                Some(operation.id),
            )
            .await
        {
            return Ok(Outcome::Crashed);
        }
        if let Err(error) = writer.upload(&mut upload).await {
            return common::fail(
                &db,
                &req.context,
                &operation,
                &upload,
                error,
                faults.as_ref(),
            )
            .await;
        }
        if !faults::hit(faults.as_ref(), FaultPoint::AfterUpload, Some(operation.id)).await
            || !faults::hit(
                faults.as_ref(),
                FaultPoint::BeforePublish,
                Some(operation.id),
            )
            .await
        {
            return Ok(Outcome::Crashed);
        }
        match writer.publish(&operation, &upload, &snapshot).await {
            Ok(Publication::Written(answer)) => {
                if !faults::hit(
                    faults.as_ref(),
                    FaultPoint::AfterPublish,
                    Some(operation.id),
                )
                .await
                {
                    return Ok(Outcome::Crashed);
                }
                return Ok(Outcome::Done(answer));
            }
            Ok(Publication::Unknown(error)) => return Err(error),
            Err(error) => {
                return common::fail(
                    &db,
                    &req.context,
                    &operation,
                    &upload,
                    error,
                    faults.as_ref(),
                )
                .await;
            }
            Ok(Publication::Conflict) => (),
        }
        if attempt + 1 >= state.config.max_retry_attempts {
            return common::fail(
                &db,
                &req.context,
                &operation,
                &upload,
                ApiError::new(ErrorCode::GenerationConflict, ""),
                faults.as_ref(),
            )
            .await;
        }
        let next = async {
            let Observed::Snapshot(next) = writer.snapshot(Some(&operation)).await? else {
                return Err(unavailable());
            };
            if !faults::hit(faults.as_ref(), FaultPoint::AfterPeek, Some(operation.id)).await
                || !faults::hit(faults.as_ref(), FaultPoint::BeforeGet, Some(operation.id)).await
            {
                return Ok(None);
            }
            let prepared = writer.prepare(&next).await?;
            Ok(Some((next, prepared)))
        }
        .await;
        let (next, prepared) = match next {
            Ok(Some(value)) => value,
            Ok(None) => return Ok(Outcome::Crashed),
            Err(error) => {
                return common::fail(
                    &db,
                    &req.context,
                    &operation,
                    &upload,
                    error,
                    faults.as_ref(),
                )
                .await;
            }
        };
        if !faults::hit(
            faults.as_ref(),
            FaultPoint::BeforeReserve,
            Some(operation.id),
        )
        .await
        {
            return Ok(Outcome::Crashed);
        }
        match writer.swap(&operation, &upload, prepared).await {
            Ok(Swap::Reserved(next_upload)) => {
                snapshot = next;
                upload = next_upload;
            }
            Ok(Swap::Failed(error) | Swap::Unknown(error)) => return Err(error),
            Ok(Swap::Crashed) => return Ok(Outcome::Crashed),
            Err(error) => {
                return common::fail(
                    &db,
                    &req.context,
                    &operation,
                    &upload,
                    error,
                    faults.as_ref(),
                )
                .await;
            }
        }
    }
    Err(unavailable())
}
