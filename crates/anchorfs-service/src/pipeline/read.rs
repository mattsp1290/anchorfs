use super::{
    faults::{self, FaultInjector, FaultPoint},
    runner::{Db, Deadline, Outcome, unavailable},
};
use crate::{
    app::AppState,
    dto::ReadQuery,
    error::{self, ApiError},
    extract::SessionAuthority,
    receipt,
    v1::{self, Gate},
};
use anchorfs_backend::key::ObjectKey;
use anchorfs_core::{
    ChunkGeometry, HandleId, ManifestId, ObjectId, ReadPinId, Sha256Digest, WorkspaceId, plan_read,
};
use anchorfs_persistence::{authority::LeaseMode, handles, manifests, namespace, pins};
use bytes::Bytes;
use std::sync::Arc;

pub struct ReadRequest {
    pub workspace: WorkspaceId,
    pub handle: HandleId,
    pub authority: SessionAuthority,
    pub query: ReadQuery,
}
pub struct ReadResult {
    pub bytes: Bytes,
    pub generation: i64,
}
pub async fn read(
    db: Db,
    deadline: Deadline,
    faults: Arc<dyn FaultInjector>,
    state: AppState,
    req: ReadRequest,
) -> Result<Outcome<ReadResult>, ApiError> {
    let mut tx = db.begin().await?;
    let p = v1::prologue(
        &mut tx,
        &req.authority.proof,
        req.workspace,
        &req.authority.binding,
        req.authority.workflow.as_ref(),
        Gate {
            role_mutation: false,
            lease: LeaseMode::Read,
        },
    )
    .await?;
    let handle =
        handles::load_for_read(&mut tx, req.workspace, &p.session, req.handle, None).await?;
    let inode = namespace::load_inode(&mut tx, req.workspace, handle.inode_id).await?;
    let (manifest, generation) = if let Some(pin) = req.query.read_pin {
        let (_, manifest) =
            pins::load_for_read(&mut tx, req.workspace, &p.session, ReadPinId(pin), inode.id)
                .await?;
        let generation = manifest.generation;
        (manifest, generation)
    } else {
        (
            manifests::current(&mut tx, req.workspace, &inode).await?,
            inode.generation,
        )
    };
    let geometry = ChunkGeometry {
        chunk_size: manifest.chunk_size.try_into().map_err(|_| unavailable())?,
        max_file_size: manifest.size as u64,
    };
    let indices: Vec<_> = plan_read(
        geometry,
        manifest.size as u64,
        req.query.offset,
        req.query.length,
        &[],
    )
    .iter()
    .map(|p| p.index as i32)
    .collect();
    let rows = manifests::chunks(
        &mut tx,
        req.workspace,
        ManifestId(manifest.id),
        Some(&indices),
    )
    .await?;
    let existing = rows
        .iter()
        .map(super::write::existing)
        .collect::<Result<Vec<_>, _>>()?;
    let plan = plan_read(
        geometry,
        manifest.size as u64,
        req.query.offset,
        req.query.length,
        &existing,
    );
    receipt::Authority {
        actor: &p.actor,
        session: Some(&p.session),
        workflow: Some(receipt::WorkflowAuthority {
            workspace: &p.workspace,
            proof: req.authority.workflow.as_ref(),
            mode: LeaseMode::Read,
        }),
    }
    .check_publication(&mut tx)
    .await?;
    tx.commit().await?;
    if !faults::hit(faults.as_ref(), FaultPoint::AfterPeek, None).await
        || !faults::hit(faults.as_ref(), FaultPoint::BeforeGet, None).await
    {
        return Ok(Outcome::Crashed);
    }
    let mut bytes = Vec::with_capacity(req.query.length as usize);
    for piece in plan {
        if let Some(row) = piece.row {
            let stored = rows
                .iter()
                .find(|r| r.chunk_index as u32 == row.index)
                .ok_or_else(unavailable)?;
            let digest = Sha256Digest(
                stored
                    .sha256
                    .as_slice()
                    .try_into()
                    .map_err(|_| unavailable())?,
            );
            let key = ObjectKey::new(
                &state.config.deployment_id,
                req.workspace,
                ObjectId(stored.object_id),
            );
            let object = deadline
                .within(
                    state
                        .backend
                        .get_verified(&key, digest, row.object_length as u64),
                )
                .await
                .map_err(|_| unavailable())?
                .map_err(error::backend)?;
            bytes.extend_from_slice(
                &object[piece.chunk_range.start as usize..piece.chunk_range.end as usize],
            );
        }
        bytes.resize(bytes.len() + piece.zero_fill as usize, 0);
    }
    Ok(Outcome::Done(ReadResult {
        bytes: bytes.into(),
        generation,
    }))
}
