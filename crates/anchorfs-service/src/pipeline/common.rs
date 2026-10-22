use super::{
    faults::{self, FaultInjector, FaultPoint},
    runner::{Db, Outcome, unavailable},
};
use crate::{
    app::AppState,
    error::ApiError,
    extract::SessionAuthority,
    receipt::{self, Answer},
    v1::{self, Gate, Prologue},
};
use anchorfs_core::{
    ChunkGeometry, ErrorCode, ExistingChunk, HandleId, Limits, ObjectId, OperationId,
    ReservationId, Secret, Sha256Digest, WorkspaceId,
};
use anchorfs_persistence::{
    authority::LeaseMode,
    handles,
    manifests::ChunkRow,
    objects, quota,
    receipts::{self, ReceiptEntry, ReceiptRequest, ReceiptScope},
    reservations::{self, ReservationError, ReservationRow},
};
use bytes::Bytes;
use sqlx::{Postgres, Transaction};

pub struct Context {
    pub workspace: WorkspaceId,
    pub authority: SessionAuthority,
    pub key: String,
    pub digest: [u8; 32],
    pub op_type: &'static str,
}
impl Context {
    pub async fn resume(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        operation: &Operation,
    ) -> Result<Prologue, ApiError> {
        v1::resume_prologue(
            tx,
            &self.authority.proof,
            self.workspace,
            &self.authority.binding,
            self.authority.workflow.as_ref(),
            Gate {
                role_mutation: true,
                lease: LeaseMode::Mutation,
            },
            operation.credential_epoch,
        )
        .await
    }
    pub async fn prologue(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Prologue, ApiError> {
        v1::prologue(
            tx,
            &self.authority.proof,
            self.workspace,
            &self.authority.binding,
            self.authority.workflow.as_ref(),
            Gate {
                role_mutation: true,
                lease: LeaseMode::Mutation,
            },
        )
        .await
    }
    pub async fn check(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        p: &Prologue,
    ) -> Result<(), ApiError> {
        receipt::Authority {
            actor: &p.actor,
            session: Some(&p.session),
            workflow: Some(receipt::WorkflowAuthority {
                workspace: &p.workspace,
                proof: self.authority.workflow.as_ref(),
                mode: LeaseMode::Mutation,
            }),
        }
        .check_publication(tx)
        .await
    }
    /// Peek-driven fences run after the read transaction ended and commit even
    /// when their resulting HTTP answer is an error.
    pub async fn resolve(&self, db: &Db, entry: ReceiptEntry) -> Result<Answer, ApiError> {
        let operation_id = match &entry {
            ReceiptEntry::Fresh { operation_id, .. }
            | ReceiptEntry::Replay { operation_id, .. }
            | ReceiptEntry::Mismatch { operation_id }
            | ReceiptEntry::Expired { operation_id, .. }
            | ReceiptEntry::Pending { operation_id, .. }
            | ReceiptEntry::Failed { operation_id } => *operation_id,
        };
        self.resolve_inner(db, entry).await.map_err(|mut error| {
            error.operation_id = Some(operation_id);
            error
        })
    }
    async fn resolve_inner(&self, db: &Db, entry: ReceiptEntry) -> Result<Answer, ApiError> {
        match entry {
            ReceiptEntry::Expired { operation_id, .. }
            | ReceiptEntry::Pending {
                operation_id,
                reservation_overdue: true,
            } => {
                let mut tx = db.begin_final().await?;
                let view = receipts::read_by_id(
                    &mut tx,
                    receipts::ReceiptFilter {
                        workspace: Some(self.workspace),
                        credential: self.authority.proof.id,
                        allow_workspace_actors: false,
                    },
                    operation_id,
                )
                .await?
                .ok_or_else(unavailable)?;
                tx.commit().await?;
                let followups = receipt::expiry_followups(Some(self.workspace), view.tombstoned);
                let mut result = match view.state.as_str() {
                    "committed" => Answer::metadata(view.result.ok_or_else(unavailable)?),
                    "failed" => Err(operation_error(ErrorCode::OperationFailed, operation_id)),
                    "indeterminate" => Err(operation_error(
                        ErrorCode::OperationIndeterminate,
                        operation_id,
                    )),
                    "pending" => Err(pending_error(operation_id)),
                    _ => Err(unavailable()),
                };
                match &mut result {
                    Ok(answer) => answer.followups = followups,
                    Err(error) => error.followups = followups,
                };
                result
            }
            ReceiptEntry::Replay { result, .. } => Answer::metadata(result),
            ReceiptEntry::Mismatch { operation_id } => Err(operation_error(
                ErrorCode::IdempotencyMismatch,
                operation_id,
            )),
            ReceiptEntry::Failed { operation_id } => {
                Err(operation_error(ErrorCode::OperationFailed, operation_id))
            }
            ReceiptEntry::Pending { operation_id, .. } => Err(pending_error(operation_id)),
            ReceiptEntry::Fresh { .. } => Err(unavailable()),
        }
    }
}
pub fn scope(p: &Prologue) -> ReceiptScope {
    ReceiptScope::Workspace {
        workspace: p.session.workspace_id,
        credential: p.actor.id,
        session: p.session.id,
        session_epoch: p.session.session_epoch,
        authorization_epoch: p.workspace.authorization_epoch,
        mutation_epoch: p.workspace.mutation_epoch,
    }
}
pub fn geometry(state: &AppState) -> ChunkGeometry {
    ChunkGeometry::from_limits(&Limits {
        chunk_size: state.config.chunk_size,
        ..Limits::default()
    })
}
pub fn operation_error(code: ErrorCode, id: OperationId) -> ApiError {
    let mut error = ApiError::new(code, "");
    error.operation_id = Some(id);
    error
}
pub fn pending_error(id: OperationId) -> ApiError {
    let mut error = operation_error(ErrorCode::OutcomeUnknown, id);
    error.retry_after_ms = Some(100);
    error
}
/// A transport failure may arrive as a PostgreSQL FATAL message rather than
/// sqlx::Error::Io. Once COMMIT has been sent, it must not trigger finalization
/// of a publication whose durable outcome has not been observed.
pub fn commit_outcome_unknown(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(error) = error else {
        return true;
    };
    if error
        .code()
        .is_some_and(|code| code.starts_with("08") || code == "40003")
    {
        return true;
    }
    error
        .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
        .is_none_or(|error| {
            matches!(
                error.severity(),
                sqlx::postgres::PgSeverity::Fatal | sqlx::postgres::PgSeverity::Panic
            )
        })
}
pub struct Operation {
    pub id: OperationId,
    pub owner: Secret<String>,
    pub scope: ReceiptScope,
    pub credential_epoch: i64,
}
pub enum Admission {
    Owned(Operation, Upload),
    Receipt(ReceiptEntry),
    Unknown(ApiError),
}
/// Writes stage their materialized chunks at admission; streamed replacement
/// reserves its declared length and stages each identity as the body arrives.
pub enum AdmissionPlan<'a> {
    Write {
        handle: HandleId,
        chunks: &'a [Prepared],
    },
    Replace {
        length: u64,
    },
}
pub async fn admit(
    db: &Db,
    ctx: &Context,
    state: &AppState,
    plan: AdmissionPlan<'_>,
) -> Result<Admission, ApiError> {
    for pressure_retry in 0..2 {
        let mut tx = db.begin().await?;
        let p = ctx.prologue(&mut tx).await?;
        let scope = scope(&p);
        let entry = receipts::begin(
            &mut tx,
            scope,
            ReceiptRequest {
                key: &ctx.key,
                op_type: ctx.op_type,
                digest: ctx.digest,
                credential_epoch: p.actor.epoch,
                retention: state.config.receipt_retention,
                wait_bound: state.config.db_lock_timeout,
            },
        )
        .await?;
        let ReceiptEntry::Fresh {
            operation_id,
            owner,
        } = entry
        else {
            ctx.check(&mut tx, &p).await?;
            tx.commit().await?;
            return Ok(Admission::Receipt(entry));
        };
        let reserved = match &plan {
            AdmissionPlan::Write { handle, chunks } => {
                handles::load_for_write(&mut tx, ctx.workspace, &p.session, *handle, None).await?;
                reserve(&mut tx, state, &p, operation_id, chunks).await
            }
            AdmissionPlan::Replace { length } => {
                reserve_parts(&mut tx, state, &p, operation_id, *length as i64, &[]).await
            }
        };
        let upload = match reserved {
            Ok(upload) => upload,
            Err(error) => {
                let pressure = matches!(error, ReservationError::Exceeded(_));
                tx.rollback().await?;
                if pressure && pressure_retry == 0 {
                    reservations::fence_overdue(
                        &db.pool,
                        Some(ctx.workspace),
                        64,
                        state.config.db_lock_timeout,
                    )
                    .await?;
                    reservations::fence_overdue(&db.pool, None, 64, state.config.db_lock_timeout)
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
                    upload,
                ));
            }
            Err(error) if !commit_outcome_unknown(&error) => return Err(error.into()),
            Err(_) => return Ok(Admission::Unknown(pending_error(operation_id))),
        }
    }
    Err(unavailable())
}

pub fn existing_chunk(row: &ChunkRow) -> Result<ExistingChunk, ApiError> {
    Ok(ExistingChunk {
        index: row.chunk_index.try_into().map_err(|_| unavailable())?,
        logical_length: row.logical_length.try_into().map_err(|_| unavailable())?,
        object_length: row.object_length.try_into().map_err(|_| unavailable())?,
    })
}
#[derive(Clone)]
pub struct Prepared {
    pub index: u32,
    pub object: ObjectId,
    pub bytes: Bytes,
    pub length: u32,
    pub digest: Sha256Digest,
}
pub struct Upload {
    pub reservation: ReservationRow,
    pub chunks: Vec<Prepared>,
    pub attempted: usize,
    pub confirmed: usize,
}
pub async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    p: &Prologue,
    operation: OperationId,
    chunks: &[Prepared],
) -> Result<Upload, ReservationError> {
    let bytes = chunks.iter().map(|c| i64::from(c.length)).sum();
    reserve_parts(tx, state, p, operation, bytes, chunks).await
}
async fn reserve_parts(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    p: &Prologue,
    operation: OperationId,
    bytes: i64,
    chunks: &[Prepared],
) -> Result<Upload, ReservationError> {
    let reservation = reservations::insert(
        tx,
        p.session.workspace_id,
        operation,
        &p.actor,
        bytes,
        state.config.upload_reservation_ttl,
    )
    .await?;
    for chunk in chunks {
        objects::stage(
            tx,
            p.session.workspace_id,
            ReservationId(reservation.id),
            operation,
            chunk.object,
            chunk.length,
            chunk.digest,
        )
        .await?;
    }
    reservations::charge(tx, p.session.workspace_id, &reservation).await?;
    Ok(Upload {
        reservation,
        chunks: chunks.to_vec(),
        attempted: 0,
        confirmed: 0,
    })
}
pub fn failure_code(error: &ApiError) -> &str {
    if error.message == anchorfs_core::error::RESERVATION_EXPIRED {
        anchorfs_core::error::RESERVATION_EXPIRED
    } else {
        error.code.as_str()
    }
}
pub async fn orphan(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    operation: &Operation,
    upload: &Upload,
    reason: &str,
) -> Result<(), ApiError> {
    let reservation = reservations::lock_for_finalize(
        tx,
        workspace,
        ReservationId(upload.reservation.id),
        operation.id,
    )
    .await?;
    reservations::fail(tx, workspace, &reservation, "failed").await?;
    let confirmed: Vec<_> = upload.chunks[..upload.confirmed]
        .iter()
        .map(|c| c.object)
        .collect();
    let uncertain: Vec<_> = upload.chunks[upload.confirmed..upload.attempted]
        .iter()
        .map(|c| c.object)
        .collect();
    let skip: Vec<_> = upload.chunks[upload.attempted..]
        .iter()
        .map(|c| c.object)
        .collect();
    objects::mark_uploaded(tx, workspace, &confirmed).await?;
    let charged = objects::orphan(
        tx,
        workspace,
        ReservationId(reservation.id),
        &uncertain,
        &skip,
        reason,
    )
    .await?;
    quota::release_to_orphans(tx, workspace, reservation.reserved_bytes, charged).await?;
    Ok(())
}
pub async fn finalize_in(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    operation: &Operation,
    upload: &Upload,
    code: &str,
) -> Result<(), ApiError> {
    receipts::lock_pending(tx, operation.scope, operation.id, &operation.owner).await?;
    orphan(tx, workspace, operation, upload, code).await?;
    if !receipts::fail_result(tx, operation.scope, operation.id, code).await? {
        return Err(unavailable());
    }
    Ok(())
}
pub async fn fail<T>(
    db: &Db,
    ctx: &Context,
    operation: &Operation,
    upload: &Upload,
    mut error: ApiError,
    faults: &dyn FaultInjector,
) -> Result<Outcome<T>, ApiError> {
    error.operation_id = Some(operation.id);
    if error.code == ErrorCode::OperationFailed {
        return Err(error);
    }
    if error.code == ErrorCode::DependencyUnavailable && error.message != "backend_integrity" {
        error.retry_after_ms = Some(100);
    }
    if !faults::hit(faults, FaultPoint::BeforeFinalize, Some(operation.id)).await {
        return Ok(Outcome::Crashed);
    }
    let cleanup = async {
        let mut tx = db.begin_final().await?;
        finalize_in(
            &mut tx,
            ctx.workspace,
            operation,
            upload,
            failure_code(&error),
        )
        .await?;
        tx.commit().await?;
        Ok::<_, ApiError>(())
    }
    .await;
    if cleanup.is_err() {
        tracing::warn!(operation_id=%operation.id,"byte finalization deferred to lazy reservation fence");
    }
    error.operation_id = Some(operation.id);
    Err(error)
}
