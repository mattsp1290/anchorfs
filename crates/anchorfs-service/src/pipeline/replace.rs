//! Conditional replacement consumes one chunk at a time, stages each identity
//! before PUT and publishes the complete manifest in a final short transaction.
use super::{
    common::{self, Context, Operation, Prepared, Upload},
    faults::{self, FaultInjector, FaultPoint},
    runner::{Db, Deadline, Outcome, PhaseCell, unavailable},
};
use crate::{
    app::AppState, dto::MutationResult, error::ApiError, extract::BinaryStream, receipt::Answer,
};
use anchorfs_backend::key::ObjectKey;
use anchorfs_core::{
    Access, ErrorCode, ObjectId, ReservationId, Sha256Digest, WorkspacePath, plan_replace, sha256,
};
use anchorfs_persistence::{
    manifests::{self, NewChunk},
    namespace::{self, InodeRow},
    objects, quota,
    receipts::{self, ReceiptEntry, ReceiptRequest},
    reservations::{self, ReservationError},
};
use bytes::Bytes;
use futures_core::Stream;
use sha2::{Digest, Sha256};
use std::{pin::Pin, sync::Arc};

pub struct ReplaceRequest {
    pub context: Context,
    pub path: WorkspacePath,
    pub expected_generation: i64,
    pub content: BinaryStream,
}
enum Snapshot {
    File(Box<InodeRow>),
    Receipt(ReceiptEntry),
}
enum Admission {
    Owned(Operation, Upload),
    Receipt(ReceiptEntry),
    Unknown(ApiError),
}
enum Published {
    Done(Answer),
    Unknown(ApiError),
}
struct Replacer<'a> {
    db: &'a Db,
    state: &'a AppState,
    context: &'a Context,
    path: &'a WorkspacePath,
    generation: i64,
    length: u64,
}
impl Replacer<'_> {
    async fn snapshot(&self) -> Result<Snapshot, ApiError> {
        let mut tx = self.db.begin().await?;
        let p = self.context.prologue(&mut tx).await?;
        if let Some(entry) = receipts::peek(
            &mut tx,
            common::scope(&p),
            &self.context.key,
            self.context.digest,
        )
        .await?
        {
            self.context.check(&mut tx, &p).await?;
            tx.commit().await?;
            return Ok(Snapshot::Receipt(entry));
        }
        plan_replace(common::geometry(self.state), self.length)?;
        let inode =
            namespace::resolve(&mut tx, self.context.workspace, Some(self.path), &p.caller).await?;
        self.validate(&inode, &p.caller)?;
        let manifest = manifests::current(&mut tx, self.context.workspace, &inode).await?;
        if manifest.chunk_size as u32 != self.state.config.chunk_size {
            return Err(ApiError::new(ErrorCode::Unsupported, ""));
        }
        quota::check_growth(
            &mut tx,
            self.context.workspace,
            self.length as i64 - inode.size,
        )
        .await?;
        self.context.check(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(Snapshot::File(Box::new(inode)))
    }
    fn validate(&self, inode: &InodeRow, caller: &namespace::Caller) -> Result<(), ApiError> {
        if inode.kind != "file" {
            return Err(ApiError::new(
                if inode.kind == "directory" {
                    ErrorCode::IsDirectory
                } else {
                    ErrorCode::InvalidInput
                },
                "",
            ));
        }
        namespace::access(
            inode,
            caller,
            Access {
                read: false,
                write: true,
                exec: false,
            },
        )?;
        if inode.generation != self.generation {
            return Err(ApiError::new(ErrorCode::GenerationConflict, ""));
        }
        Ok(())
    }
    async fn admit(&self) -> Result<Admission, ApiError> {
        for retry in 0..2 {
            let mut tx = self.db.begin().await?;
            let p = self.context.prologue(&mut tx).await?;
            let scope = common::scope(&p);
            let entry = receipts::begin(
                &mut tx,
                scope,
                ReceiptRequest {
                    key: &self.context.key,
                    op_type: self.context.op_type,
                    digest: self.context.digest,
                    credential_epoch: p.actor.epoch,
                    retention: self.state.config.receipt_retention,
                    wait_bound: self.state.config.db_lock_timeout,
                },
            )
            .await?;
            let ReceiptEntry::Fresh {
                operation_id,
                owner,
            } = entry
            else {
                self.context.check(&mut tx, &p).await?;
                tx.commit().await?;
                return Ok(Admission::Receipt(entry));
            };
            let reserved = async {
                let row = reservations::insert(
                    &mut tx,
                    self.context.workspace,
                    operation_id,
                    &p.actor,
                    self.length as i64,
                    self.state.config.upload_reservation_ttl,
                )
                .await?;
                reservations::charge(&mut tx, self.context.workspace, &row).await?;
                Ok::<_, ReservationError>(row)
            }
            .await;
            let reservation = match reserved {
                Ok(row) => row,
                Err(error) => {
                    let pressure = matches!(error, ReservationError::Exceeded(_));
                    tx.rollback().await?;
                    if pressure && retry == 0 {
                        reservations::fence_overdue(
                            &self.db.pool,
                            Some(self.context.workspace),
                            64,
                            self.state.config.db_lock_timeout,
                        )
                        .await?;
                        reservations::fence_overdue(
                            &self.db.pool,
                            None,
                            64,
                            self.state.config.db_lock_timeout,
                        )
                        .await?;
                        continue;
                    }
                    return Err(error.into());
                }
            };
            match tx.commit().await {
                Ok(()) => {
                    return Ok(Admission::Owned(
                        Operation {
                            id: operation_id,
                            owner,
                            scope,
                            credential_epoch: p.actor.epoch,
                        },
                        Upload {
                            reservation,
                            chunks: Vec::new(),
                            attempted: 0,
                            confirmed: 0,
                        },
                    ));
                }
                Err(error) if !common::commit_outcome_unknown(&error) => {
                    return Err(error.into());
                }
                Err(_) => return Ok(Admission::Unknown(common::pending_error(operation_id))),
            }
        }
        Err(unavailable())
    }
    async fn publish(
        &self,
        operation: &Operation,
        upload: &Upload,
        snapshot: &InodeRow,
    ) -> Result<Published, ApiError> {
        let mut tx = self.db.begin_final().await?;
        let p = self.context.resume(&mut tx, operation).await?;
        let op = receipts::lock_pending(&mut tx, operation.scope, operation.id, &operation.owner)
            .await?;
        if op.authorization_epoch_at_start != p.workspace.authorization_epoch
            || op.mutation_epoch_at_start != p.workspace.mutation_epoch
        {
            return Err(ApiError::new(ErrorCode::Fenced, ""));
        }
        let inode = namespace::lock_inodes(&mut tx, self.context.workspace, &[snapshot.id])
            .await?
            .pop()
            .ok_or_else(unavailable)?;
        if inode.generation != self.generation {
            return Err(ApiError::new(ErrorCode::GenerationConflict, ""));
        }
        let resolved =
            namespace::resolve(&mut tx, self.context.workspace, Some(self.path), &p.caller).await?;
        if resolved.id != inode.id || inode.nlink == 0 {
            return Err(ApiError::new(ErrorCode::NotFound, ""));
        }
        self.validate(&inode, &p.caller)?;
        let reservation = reservations::lock_active(
            &mut tx,
            self.context.workspace,
            ReservationId(upload.reservation.id),
            operation.id,
        )
        .await?;
        let ids: Vec<_> = upload.chunks.iter().map(|c| c.object).collect();
        objects::mark_uploaded(&mut tx, self.context.workspace, &ids).await?;
        let published = objects::publish(
            &mut tx,
            self.context.workspace,
            ReservationId(reservation.id),
        )
        .await?;
        if published != self.length as i64 {
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
        let (_, next) = manifests::publish_replace(
            &mut tx,
            self.context.workspace,
            &inode,
            operation.id,
            self.state.config.chunk_size,
            self.length,
            &chunks,
        )
        .await?;
        reservations::consume(&mut tx, self.context.workspace, &reservation).await?;
        quota::settle(
            &mut tx,
            self.context.workspace,
            reservation.reserved_bytes,
            published,
            next.size - inode.size,
        )
        .await?;
        let result = serde_json::to_value(MutationResult {
            operation_id: operation.id.0,
            inode_id: Some(inode.id),
            generation: Some(next.generation),
        })
        .map_err(|_| unavailable())?;
        if !receipts::commit_result(&mut tx, operation.scope, operation.id, &result).await? {
            return Err(ApiError::new(ErrorCode::OperationFailed, ""));
        }
        self.context.check(&mut tx, &p).await?;
        match tx.commit().await {
            Ok(()) => Ok(Published::Done(Answer::metadata(result)?)),
            Err(error) if !common::commit_outcome_unknown(&error) => Err(error.into()),
            Err(_) => Ok(Published::Unknown(common::pending_error(operation.id))),
        }
    }
    /// Empty replacement never commits a visible pending row or reservation.
    async fn empty(
        &self,
        snapshot: &InodeRow,
        validation: Result<(), ApiError>,
        phase: &PhaseCell,
        faults: &dyn FaultInjector,
    ) -> Result<Outcome<Answer>, ApiError> {
        if !phase.start_admission() {
            return Err(unavailable());
        }
        let mut tx = self.db.begin_final().await?;
        let p = self.context.prologue(&mut tx).await?;
        let scope = common::scope(&p);
        let entry = receipts::begin(
            &mut tx,
            scope,
            ReceiptRequest {
                key: &self.context.key,
                op_type: self.context.op_type,
                digest: self.context.digest,
                credential_epoch: p.actor.epoch,
                retention: self.state.config.receipt_retention,
                wait_bound: self.state.config.db_lock_timeout,
            },
        )
        .await?;
        let ReceiptEntry::Fresh { operation_id, .. } = entry else {
            self.context.check(&mut tx, &p).await?;
            tx.commit().await?;
            return self
                .context
                .resolve(self.db, entry)
                .await
                .map(Outcome::Done);
        };
        let answer = match validation {
            Err(mut error) => {
                if !receipts::fail_result(&mut tx, scope, operation_id, error.code.as_str()).await?
                {
                    return Err(unavailable());
                }
                error.operation_id = Some(operation_id);
                Err(error)
            }
            Ok(()) => {
                let inode = namespace::lock_inodes(&mut tx, self.context.workspace, &[snapshot.id])
                    .await?
                    .pop()
                    .ok_or_else(unavailable)?;
                self.validate(&inode, &p.caller)?;
                let resolved =
                    namespace::resolve(&mut tx, self.context.workspace, Some(self.path), &p.caller)
                        .await?;
                if resolved.id != inode.id || inode.nlink == 0 {
                    return Err(ApiError::new(ErrorCode::NotFound, ""));
                }
                let (_, next) = manifests::publish_replace(
                    &mut tx,
                    self.context.workspace,
                    &inode,
                    operation_id,
                    self.state.config.chunk_size,
                    0,
                    &[],
                )
                .await?;
                quota::apply(
                    &mut tx,
                    self.context.workspace,
                    quota::Charge {
                        inodes: 0,
                        bytes: -inode.size,
                    },
                )
                .await?;
                let result = serde_json::to_value(MutationResult {
                    operation_id: operation_id.0,
                    inode_id: Some(inode.id),
                    generation: Some(next.generation),
                })
                .map_err(|_| unavailable())?;
                if !receipts::commit_result(&mut tx, scope, operation_id, &result).await? {
                    return Err(unavailable());
                }
                Answer::metadata(result)
            }
        };
        self.context.check(&mut tx, &p).await?;
        match tx.commit().await {
            Ok(()) => {
                phase.admitted(operation_id);
                if answer.is_ok()
                    && !faults::hit(faults, FaultPoint::AfterPublish, Some(operation_id)).await
                {
                    return Ok(Outcome::Crashed);
                }
                answer.map(Outcome::Done)
            }
            Err(error) if !common::commit_outcome_unknown(&error) => Err(error.into()),
            Err(_) => Err(common::pending_error(operation_id)),
        }
    }
}
struct ContentReader {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, axum::Error>> + Send>>,
    remainder: Bytes,
    deadline: Deadline,
}
impl ContentReader {
    fn new(body: axum::body::Body, deadline: Deadline) -> Self {
        Self {
            stream: Box::pin(body.into_data_stream()),
            remainder: Bytes::new(),
            deadline,
        }
    }
    async fn next(&mut self) -> Result<Option<Bytes>, ApiError> {
        if tokio::time::Instant::now() >= self.deadline.backend_by {
            return Err(unavailable());
        }
        self.deadline
            .within(std::future::poll_fn(|cx| {
                self.stream.as_mut().poll_next(cx)
            }))
            .await
            .map_err(|_| unavailable())?
            .transpose()
            .map_err(stream_error)
    }
    async fn chunk(&mut self, length: u32) -> Result<Bytes, ApiError> {
        let mut bytes = Vec::with_capacity(length as usize);
        while bytes.len() < length as usize {
            if self.remainder.is_empty() {
                self.remainder = self
                    .next()
                    .await?
                    .ok_or_else(|| invalid("content_length_mismatch"))?;
                if self.remainder.is_empty() {
                    continue;
                }
            }
            let take = (length as usize - bytes.len()).min(self.remainder.len());
            bytes.extend_from_slice(&self.remainder.split_to(take));
        }
        Ok(bytes.into())
    }
    async fn finish(&mut self) -> Result<(), ApiError> {
        if !self.remainder.is_empty() {
            return Err(invalid("content_length_mismatch"));
        }
        while let Some(bytes) = self.next().await? {
            if !bytes.is_empty() {
                return Err(invalid("content_length_mismatch"));
            }
        }
        Ok(())
    }
}
fn stream_error(error: axum::Error) -> ApiError {
    let mut source: &(dyn std::error::Error + 'static) = &error;
    loop {
        if source.is::<http_body_util::LengthLimitError>() {
            return invalid("content_length_mismatch");
        }
        match source.source() {
            Some(inner) => source = inner,
            None => return unavailable(),
        }
    }
}
fn invalid(message: &str) -> ApiError {
    let mut error = ApiError::new(ErrorCode::InvalidInput, "");
    error.message = message.into();
    error
}

pub async fn replace(
    db: Db,
    deadline: Deadline,
    faults: Arc<dyn FaultInjector>,
    phase: Arc<PhaseCell>,
    state: AppState,
    req: ReplaceRequest,
) -> Result<Outcome<Answer>, ApiError> {
    let replacer = Replacer {
        db: &db,
        state: &state,
        context: &req.context,
        path: &req.path,
        generation: req.expected_generation,
        length: req.content.length,
    };
    let snapshot = match replacer.snapshot().await? {
        Snapshot::File(inode) => inode,
        Snapshot::Receipt(entry) => {
            return req.context.resolve(&db, entry).await.map(Outcome::Done);
        }
    };
    if !faults::hit(faults.as_ref(), FaultPoint::AfterPeek, None).await {
        return Ok(Outcome::Crashed);
    }
    let mut reader = ContentReader::new(req.content.body, deadline);
    if req.content.length == 0 {
        let validation = async {
            reader.finish().await?;
            if req.content.sha256 != sha256([]) {
                return Err(invalid("content_digest_mismatch"));
            }
            Ok(())
        }
        .await;
        if validation.is_ok()
            && !faults::hit(faults.as_ref(), FaultPoint::BeforePublish, None).await
        {
            return Ok(Outcome::Crashed);
        }
        return replacer
            .empty(&snapshot, validation, &phase, faults.as_ref())
            .await;
    }
    let plan = plan_replace(common::geometry(&state), req.content.length)?;
    if !faults::hit(faults.as_ref(), FaultPoint::BeforeReserve, None).await {
        return Ok(Outcome::Crashed);
    }
    if !phase.start_admission() {
        return Err(unavailable());
    }
    let (operation, mut upload) = match replacer.admit().await? {
        Admission::Owned(op, upload) => (op, upload),
        Admission::Receipt(entry) => {
            return req.context.resolve(&db, entry).await.map(Outcome::Done);
        }
        Admission::Unknown(error) => return Err(error),
    };
    phase.admitted(operation.id);
    if !faults::hit(
        faults.as_ref(),
        FaultPoint::AfterReserve,
        Some(operation.id),
    )
    .await
    {
        return Ok(Outcome::Crashed);
    }
    let mut hasher = Sha256::new();
    for (index, length) in plan {
        let result = async {
            let bytes = reader.chunk(length).await?;
            hasher.update(&bytes);
            let digest = sha256(&bytes);
            let object = ObjectId::new();
            upload.chunks.push(Prepared {
                index,
                object,
                bytes: Bytes::new(),
                length,
                digest,
            });
            let mut tx = db.begin_final().await?;
            objects::stage(
                &mut tx,
                req.context.workspace,
                ReservationId(upload.reservation.id),
                operation.id,
                object,
                length,
                digest,
            )
            .await?;
            tx.commit().await?;
            if index == 0
                && !faults::hit(
                    faults.as_ref(),
                    FaultPoint::BeforeUpload,
                    Some(operation.id),
                )
                .await
            {
                return Ok(false);
            }
            let key = ObjectKey::new(&state.config.deployment_id, req.context.workspace, object);
            upload.attempted += 1;
            deadline
                .within(state.backend.put(&key, bytes, digest))
                .await
                .map_err(|_| unavailable())?
                .map_err(|_| unavailable())?;
            upload.confirmed += 1;
            Ok::<_, ApiError>(true)
        }
        .await;
        match result {
            Ok(true) => (),
            Ok(false) => return Ok(Outcome::Crashed),
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
    let validate = async {
        reader.finish().await?;
        if Sha256Digest(hasher.finalize().into()) != req.content.sha256 {
            return Err(invalid("content_digest_mismatch"));
        }
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(error) = validate {
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
    match replacer.publish(&operation, &upload, &snapshot).await {
        Ok(Published::Done(answer)) => {
            if !faults::hit(
                faults.as_ref(),
                FaultPoint::AfterPublish,
                Some(operation.id),
            )
            .await
            {
                return Ok(Outcome::Crashed);
            }
            Ok(Outcome::Done(answer))
        }
        Ok(Published::Unknown(error)) => Err(error),
        Err(error) => {
            common::fail(
                &db,
                &req.context,
                &operation,
                &upload,
                error,
                faults.as_ref(),
            )
            .await
        }
    }
}
