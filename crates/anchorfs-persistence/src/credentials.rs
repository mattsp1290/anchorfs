//! Independently selected credential authority and issuer-tree revocation.
use crate::{
    authority::{ActorCredential, AuthorityError, CredentialKind, CredentialRole},
    rows::CredentialRow,
};
use anchorfs_core::{CredentialId, Secret, WorkspaceId, random_token, secret_hash};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;

pub struct NewCredential {
    pub workspace_id: Option<WorkspaceId>,
    pub kind: CredentialKind,
    pub role: CredentialRole,
    pub uid: u32,
    pub gid: u32,
    pub expires_at: Option<OffsetDateTime>,
    pub max_reservation_bytes: i64,
}
pub struct Issued {
    pub id: CredentialId,
    pub secret: Secret<String>,
    pub epoch: i64,
    pub expires_at: Option<OffsetDateTime>,
}
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("administrator already exists")]
    AdministratorExists,
    #[error("invalid administrator parameters")]
    InvalidInput,
    #[error("database unavailable")]
    Database(#[from] sqlx::Error),
}
impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Administrator => "administrator",
            Self::Host => "host",
            Self::Worker => "worker",
        }
    }
}
impl CredentialRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }
}
async fn future_expiry(
    tx: &mut Transaction<'_, Postgres>,
    expiry: Option<OffsetDateTime>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT $1::timestamptz IS NULL OR $1 > clock_timestamp()")
        .bind(expiry)
        .fetch_one(&mut **tx)
        .await
}
async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    new: NewCredential,
    issuer: Option<CredentialId>,
) -> Result<Issued, sqlx::Error> {
    let id = CredentialId::new();
    let secret = random_token();
    sqlx::query("INSERT INTO credentials (id,workspace_id,kind,role,uid,gid,expires_at,max_reservation_bytes,issued_by_credential_id,secret_hash,epoch,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,1,transaction_timestamp())")
        .bind(id.0).bind(new.workspace_id.map(|w| w.0)).bind(new.kind.as_str()).bind(new.role.as_str()).bind(i64::from(new.uid)).bind(i64::from(new.gid)).bind(new.expires_at).bind(new.max_reservation_bytes).bind(issuer.map(|i| i.0)).bind(secret_hash(secret.expose()).0.as_slice()).execute(&mut **tx).await?;
    Ok(Issued {
        id,
        secret,
        epoch: 1,
        expires_at: new.expires_at,
    })
}
pub async fn bootstrap_administrator(
    tx: &mut Transaction<'_, Postgres>,
    uid: u32,
    gid: u32,
    max_reservation_bytes: i64,
    expires_at: Option<OffsetDateTime>,
) -> Result<Issued, BootstrapError> {
    sqlx::query("LOCK TABLE credentials IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut **tx)
        .await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM credentials WHERE kind='administrator' AND revoked_at IS NULL)").fetch_one(&mut **tx).await?;
    if exists {
        return Err(BootstrapError::AdministratorExists);
    }
    if max_reservation_bytes <= 0 || !future_expiry(tx, expires_at).await? {
        return Err(BootstrapError::InvalidInput);
    }
    Ok(insert(
        tx,
        NewCredential {
            workspace_id: None,
            kind: CredentialKind::Administrator,
            role: CredentialRole::Admin,
            uid,
            gid,
            expires_at,
            max_reservation_bytes,
        },
        None,
    )
    .await?)
}
pub async fn issue(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    new: NewCredential,
) -> Result<Issued, AuthorityError> {
    if new.max_reservation_bytes <= 0
        || !future_expiry(tx, new.expires_at).await?
        || (new.kind == CredentialKind::Administrator
            && (new.workspace_id.is_some() || new.role != CredentialRole::Admin))
        || (new.kind != CredentialKind::Administrator
            && (new.workspace_id.is_none() || new.role == CredentialRole::Admin))
    {
        return Err(AuthorityError::InvalidInput);
    }
    match actor.kind {
        CredentialKind::Administrator => {}
        CredentialKind::Host => {
            if new.workspace_id != actor.workspace_id {
                return Err(AuthorityError::NotFound);
            }
            if new.kind != CredentialKind::Worker
                || new.uid != actor.uid
                || new.gid != actor.gid
                || new.role > actor.role
                || new.max_reservation_bytes > actor.max_reservation_bytes
                || actor
                    .expires_at
                    .is_some_and(|bound| new.expires_at.is_none_or(|expiry| expiry > bound))
            {
                return Err(AuthorityError::Forbidden);
            }
        }
        CredentialKind::Worker => return Err(AuthorityError::Forbidden),
    }
    let issued = insert(tx, new, Some(actor.id)).await?;
    crate::authority::check_publication_time(tx, actor, None).await?;
    Ok(issued)
}
/// Caller holds the workspace and actor locks before invoking target mutations.
async fn target(
    tx: &mut Transaction<'_, Postgres>,
    id: CredentialId,
) -> Result<CredentialRow, AuthorityError> {
    let row =
        sqlx::query_as::<_, CredentialRow>("SELECT * FROM credentials WHERE id=$1 FOR UPDATE")
            .bind(id.0)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(AuthorityError::NotFound)?;
    if row.revoked_at.is_some() {
        return Err(AuthorityError::NotFound);
    }
    Ok(row)
}
fn authorize_target(
    actor: &ActorCredential,
    row: &CredentialRow,
    rotation: bool,
) -> Result<(), AuthorityError> {
    if actor.kind == CredentialKind::Administrator {
        return Ok(());
    }
    if actor.workspace_id.map(|w| w.0) != row.workspace_id {
        return Err(AuthorityError::NotFound);
    }
    if actor.kind != CredentialKind::Host
        || !(row.issued_by_credential_id == Some(actor.id.0) || (rotation && row.id == actor.id.0))
    {
        return Err(AuthorityError::Forbidden);
    }
    Ok(())
}
pub async fn rotate(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    id: CredentialId,
) -> Result<Issued, AuthorityError> {
    let row = target(tx, id).await?;
    authorize_target(actor, &row, true)?;
    let secret = random_token();
    let epoch = sqlx::query_scalar(
        "UPDATE credentials SET secret_hash=$2,epoch=epoch+1 WHERE id=$1 RETURNING epoch",
    )
    .bind(id.0)
    .bind(secret_hash(secret.expose()).0.as_slice())
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("UPDATE client_sessions SET revoked_at=transaction_timestamp() WHERE credential_id=$1 AND revoked_at IS NULL").bind(id.0).execute(&mut **tx).await?;
    crate::authority::check_publication_time(tx, actor, None).await?;
    Ok(Issued {
        id,
        secret,
        epoch,
        expires_at: row.expires_at,
    })
}
pub async fn is_ancestor_or_self(
    tx: &mut Transaction<'_, Postgres>,
    candidate: CredentialId,
    of: CredentialId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("WITH RECURSIVE ancestors AS (SELECT id,issued_by_credential_id FROM credentials WHERE id=$1 UNION SELECT c.id,c.issued_by_credential_id FROM credentials c JOIN ancestors a ON c.id=a.issued_by_credential_id) SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=$2)")
        .bind(of.0).bind(candidate.0).fetch_one(&mut **tx).await
}
pub struct RevokeOutcome {
    pub revoked_credentials: Vec<CredentialId>,
    pub revoked_sessions: u64,
}
pub async fn revoke(
    tx: &mut Transaction<'_, Postgres>,
    actor: &ActorCredential,
    id: CredentialId,
) -> Result<RevokeOutcome, AuthorityError> {
    let row = target(tx, id).await?;
    authorize_target(actor, &row, false)?;
    let mut locked = vec![id.0];
    let mut frontier = vec![id.0];
    while !frontier.is_empty() {
        let children: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM credentials WHERE issued_by_credential_id=ANY($1) AND revoked_at IS NULL ORDER BY id FOR UPDATE").bind(&frontier).fetch_all(&mut **tx).await?;
        locked.extend(children.iter().copied());
        frontier = children;
    }
    sqlx::query(
        "UPDATE credentials SET revoked_at=transaction_timestamp(),epoch=epoch+1 WHERE id=ANY($1)",
    )
    .bind(&locked)
    .execute(&mut **tx)
    .await?;
    let sessions = sqlx::query("UPDATE client_sessions SET revoked_at=transaction_timestamp() WHERE credential_id=ANY($1) AND revoked_at IS NULL").bind(&locked).execute(&mut **tx).await?.rows_affected();
    crate::authority::check_publication_time(tx, actor, None).await?;
    Ok(RevokeOutcome {
        revoked_credentials: locked.into_iter().map(CredentialId).collect(),
        revoked_sessions: sessions,
    })
}
