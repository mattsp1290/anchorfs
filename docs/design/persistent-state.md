# Persistent state ownership

Schema contract version 1. [Migrations](../../crates/anchorfs-persistence/migrations/) define the database constraints. Inputs are approved specs 02, 03, 04 and 07 and the independently selected Stage 0 plan. Stage 0 recorded conceptual transactions. Stage 1a implements the control-plane transactions annotated below; lease, byte publication, quota accounting and storage recovery remain conceptual. Schema coverage alone proves only structural constraints.

Deployment-scope createWorkspace, createCredential, rotateCredential, revokeCredential and createSession use admin_operations receipts. All session-bound workspace mutations use operations. Both bind a request digest and credential epoch; workspace operations additionally bind session and workspace epochs. A receipt permits outcome retrieval, never new authority.

The issuer FK on credentials.id is the documented exception to composite tenant references: a deployment administrator has a NULL workspace. The credentials_issuer_scope trigger enforces a same-workspace non-administrator issuer. All other workspace references include workspace_id. Initial identity grants read/write/admin roles: server traversal/mode checks are Stage 1; only an owner or workspace admin may chmod, only an administrator may chown, and workers cannot hold admin role or mint sessions/leases.

## Column ownership

SQL type/nullability/checks below are structural invariants; the semantic invariant in each row is a future transaction obligation. Coverage checks enforce an exact table.column set, not the correctness of future recovery logic.

## schema_meta

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| schema_meta.singleton | deployment_initialize / restore | `boolean PRIMARY KEY CHECK (singleton)`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.schema_version | deployment_initialize / restore | `integer NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.object_format_version | deployment_initialize / restore | `integer NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.deployment_id | deployment_initialize / restore | `text NOT NULL CHECK (deployment_id ~ '^[a-z0-9-]{1,64}$')`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.created_at | deployment_initialize / restore | `timestamptz NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |

## deployment_quotas

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| deployment_quotas.singleton | deployment_initialize / write_commit / reservation_expire / offline_collect | `boolean PRIMARY KEY CHECK (singleton)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.physical_bytes_limit | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL CHECK (physical_bytes_limit > 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.staging_bytes_limit | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL CHECK (staging_bytes_limit > 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.physical_bytes_used | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used >= 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.staging_bytes_reserved | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved >= 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |

## admin_operations

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| admin_operations.id | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `uuid PRIMARY KEY`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.credential_id | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `uuid NOT NULL`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.credential_epoch_at_start | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `bigint NOT NULL`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.idempotency_key | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 256)`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.op_type | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `text NOT NULL CHECK (op_type IN ('workspace_create','credential_issue','credential_rotate','credential_revoke','session_create'))`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.request_digest | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `bytea NOT NULL CHECK (octet_length(request_digest)=32)`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.owner_token | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `bytea NOT NULL CHECK (octet_length(owner_token)=32)`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.heartbeat_at | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `timestamptz NOT NULL`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.state | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create / receipt_expire (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `text NOT NULL CHECK (state IN ('pending','committed','failed','indeterminate'))`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.result | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create / receipt_expire (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `jsonb`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.created_at | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `timestamptz`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.committed_at | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `timestamptz`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |
| admin_operations.expires_at | workspace_create / credential_issue / credential_rotate / credential_revoke / session_create (implemented: receipts::begin/commit_result/read_by_id/read_by_key) | `timestamptz NOT NULL`; credential-scoped key uniquely binds request digest and captured credential epoch | B4, B6, B11 | resolve stored result; expired ambiguous outcome requires inspection, never blind replay |

## workspaces

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| workspaces.id | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `uuid PRIMARY KEY`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.root_inode_id | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `bigint NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.state | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `text NOT NULL CHECK (state IN ('active','maintenance'))`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.authorization_epoch | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `bigint NOT NULL DEFAULT 1`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.mutation_epoch | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `bigint NOT NULL DEFAULT 1`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_owner_credential_id | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create (creation only)) | `uuid`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_token_hash | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create (creation only)) | `bytea CHECK (octet_length(workflow_token_hash)=32)`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_expires_at | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create (creation only)) | `timestamptz`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.created_at | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `timestamptz NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.updated_at | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create (creation only)) | `timestamptz NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |

## workspace_quotas

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| workspace_quotas.workspace_id | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `uuid PRIMARY KEY REFERENCES workspaces(id)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (logical_bytes_limit>0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_inodes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (logical_inodes_limit>0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.physical_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (physical_bytes_limit>0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.staging_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (staging_bytes_limit>0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_bytes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL DEFAULT 0 CHECK (logical_bytes_used>=0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_inodes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL DEFAULT 0 CHECK (logical_inodes_used>=0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.physical_bytes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used>=0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.staging_bytes_reserved | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved>=0)`; all mutations serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |

## namespace_locks

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| namespace_locks.workspace_id | workspace_create / namespace_mutation (implemented: workspaces::create (creation only)) | `uuid PRIMARY KEY REFERENCES workspaces(id)`; exclusive namespace lock serializes ancestor validation; generation binds listing cursors | B6 | use committed namespace and receipt; do not replay ambiguous rename under a new key |
| namespace_locks.namespace_generation | workspace_create / namespace_mutation (implemented: workspaces::create (creation only)) | `bigint NOT NULL DEFAULT 1`; exclusive namespace lock serializes ancestor validation; generation binds listing cursors | B6 | use committed namespace and receipt; do not replay ambiguous rename under a new key |

## credentials

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| credentials.id | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `uuid PRIMARY KEY`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.workspace_id | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `uuid REFERENCES workspaces(id)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.kind | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `text NOT NULL CHECK (kind IN ('administrator','host','worker'))`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.role | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `text NOT NULL CHECK (role IN ('read','write','admin'))`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.issued_by_credential_id | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `uuid REFERENCES credentials(id)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.uid | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `bigint NOT NULL CHECK (uid BETWEEN 0 AND 4294967295)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.gid | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `bigint NOT NULL CHECK (gid BETWEEN 0 AND 4294967295)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.secret_hash | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `bytea NOT NULL CHECK (octet_length(secret_hash)=32)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.epoch | credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `bigint NOT NULL DEFAULT 1`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.max_reservation_bytes | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `bigint NOT NULL CHECK (max_reservation_bytes>0)`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.expires_at | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `timestamptz`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.revoked_at | credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `timestamptz`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |
| credentials.created_at | credential_issue / credential_rotate / credential_revoke (implemented: credentials::issue/rotate/revoke/bootstrap_administrator) | `timestamptz NOT NULL`; identity and role are administrator-bound; revocation increments epoch atomically and cascades to issued credentials | B5, B8 | recheck current epoch and revoked_at; reject old capabilities and new sessions from revoked workers |

## client_sessions

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| client_sessions.id | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `uuid PRIMARY KEY`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.workspace_id | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `uuid NOT NULL REFERENCES workspaces(id)`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.credential_id | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `uuid NOT NULL`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.credential_epoch | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `bigint NOT NULL`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.session_epoch | session_create / session_rebind / session_revoke / receipt_expire (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `bigint NOT NULL`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.bound_mutation_epoch | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `bigint`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.expires_at | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `timestamptz NOT NULL`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.revoked_at | session_create / session_rebind / session_revoke / receipt_expire (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `timestamptz`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |

## inodes

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| inodes.workspace_id | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `uuid NOT NULL REFERENCES workspaces(id)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.id | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `bigint NOT NULL DEFAULT nextval('inodes_id_seq')`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.generation | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `bigint NOT NULL DEFAULT 1`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.kind | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `text NOT NULL CHECK (kind IN ('file','directory','symlink'))`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.size | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `bigint NOT NULL DEFAULT 0 CHECK (size>=0)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.mode | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `integer NOT NULL CHECK (mode BETWEEN 0 AND 511)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.uid | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `bigint NOT NULL CHECK (uid BETWEEN 0 AND 4294967295)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.gid | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `bigint NOT NULL CHECK (gid BETWEEN 0 AND 4294967295)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.atime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.mtime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.ctime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.nlink | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `integer NOT NULL DEFAULT 1 CHECK (nlink>=0)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.current_manifest_id | write_commit / truncate_commit (implemented: workspaces::create (root only)) | `uuid`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.symlink_target | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `text`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.unlinked_at | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create (root only)) | `timestamptz`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |

## directory_entries

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| directory_entries.workspace_id | namespace_mutation | `uuid NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.parent_inode_id | namespace_mutation | `bigint NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.name | namespace_mutation | `text COLLATE "C" NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255 AND position('/' in name)=0 AND name NOT IN ('.','..'))`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.inode_id | namespace_mutation | `bigint NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |

## upload_reservations

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| upload_reservations.workspace_id | write_commit / reservation_expire / reconcile_uncertain | `uuid NOT NULL REFERENCES workspaces(id)`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.id | write_commit / reservation_expire / reconcile_uncertain | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.operation_id | write_commit / reservation_expire / reconcile_uncertain | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.credential_id | write_commit / reservation_expire / reconcile_uncertain | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.reserved_bytes | write_commit / reservation_expire / reconcile_uncertain | `bigint NOT NULL CHECK (reserved_bytes>0)`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.state | write_commit / reservation_expire / reconcile_uncertain | `text NOT NULL CHECK (state IN ('active','consumed','expired','failed'))`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.heartbeat_at | write_commit / reservation_expire / reconcile_uncertain | `timestamptz NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.expires_at | write_commit / reservation_expire / reconcile_uncertain | `timestamptz NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |

## objects

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| objects.workspace_id | write_commit / reconcile_uncertain / offline_collect | `uuid NOT NULL REFERENCES workspaces(id)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.id | write_commit / reconcile_uncertain / offline_collect | `uuid NOT NULL`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.byte_length | write_commit / reconcile_uncertain / offline_collect | `bigint NOT NULL CHECK (byte_length>0)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.sha256 | write_commit / reconcile_uncertain / offline_collect | `bytea NOT NULL CHECK (octet_length(sha256)=32)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.state | write_commit / reconcile_uncertain / offline_collect | `text NOT NULL CHECK (state IN ('staged','uncertain','published'))`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.reservation_id | write_commit / reconcile_uncertain / offline_collect | `uuid NOT NULL`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.uploaded_at | write_commit / reconcile_uncertain / offline_collect | `timestamptz`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.published_at | write_commit / reconcile_uncertain / offline_collect | `timestamptz`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |

## manifests

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| manifests.workspace_id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.inode_id | write_commit / truncate_commit / offline_collect | `bigint NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.size | write_commit / truncate_commit / offline_collect | `bigint NOT NULL CHECK (size>=0)`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.chunk_size | write_commit / truncate_commit / offline_collect | `integer NOT NULL CHECK (chunk_size>0)`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.created_by_operation_id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.created_at | write_commit / truncate_commit / offline_collect | `timestamptz NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |

## manifest_chunks

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| manifest_chunks.workspace_id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.manifest_id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.chunk_index | write_commit / truncate_commit / offline_collect | `integer NOT NULL CHECK (chunk_index>=0)`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.chunk_size | write_commit / truncate_commit / offline_collect | `integer NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.object_id | write_commit / truncate_commit / offline_collect | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.sha256 | write_commit / truncate_commit / offline_collect | `bytea NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.logical_length | write_commit / truncate_commit / offline_collect | `integer NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |

## operations

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| operations.workspace_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.credential_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.client_session_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.session_epoch | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.credential_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.authorization_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.mutation_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.owner_token | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bytea NOT NULL CHECK (octet_length(owner_token)=32)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.heartbeat_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `timestamptz NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.idempotency_key | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 256)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.op_type | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `text NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.request_digest | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `bytea NOT NULL CHECK (octet_length(request_digest)=32)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.state | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain / receipt_expire (implemented: receipts (session_* operations only)) | `text NOT NULL CHECK (state IN ('pending','committed','failed','indeterminate'))`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.result | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain / receipt_expire (implemented: receipts (session_* operations only)) | `jsonb`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.created_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `timestamptz`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.committed_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `timestamptz`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.expires_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts (session_* operations only)) | `timestamptz NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |

## handles

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| handles.workspace_id | handle_open / handle_renew / handle_close / reservation_expire | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.id | handle_open / handle_renew / handle_close / reservation_expire | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.inode_id | handle_open / handle_renew / handle_close / reservation_expire | `bigint NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.client_session_id | handle_open / handle_renew / handle_close / reservation_expire | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.credential_id | handle_open / handle_renew / handle_close / reservation_expire | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.access_read | handle_open / handle_renew / handle_close / reservation_expire | `boolean NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.access_write | handle_open / handle_renew / handle_close / reservation_expire | `boolean NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.opened_at | handle_open / handle_renew / handle_close / reservation_expire | `timestamptz NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.expires_at | handle_open / handle_renew / handle_close / reservation_expire | `timestamptz NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.closed_at | handle_open / handle_renew / handle_close / reservation_expire | `timestamptz`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |

## read_pins

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| read_pins.workspace_id | pin_create / reservation_expire / restore | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.id | pin_create / reservation_expire / restore | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.manifest_id | pin_create / reservation_expire / restore | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.client_session_id | pin_create / reservation_expire / restore | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.expires_at | pin_create / reservation_expire / restore | `timestamptz NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |

## deletion_candidates

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| deletion_candidates.workspace_id | reconcile_uncertain / offline_collect | `uuid NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.object_id | reconcile_uncertain / offline_collect | `uuid NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.reason | reconcile_uncertain / offline_collect | `text NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.physical_bytes | reconcile_uncertain / offline_collect | `bigint NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.recorded_at | reconcile_uncertain / offline_collect | `timestamptz NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |

## Crash boundaries

All scenarios below remain unverified until their Stage 1–3 acceptance tests exist.

### B1 — Reservation committed, upload not started

operation, reservation and staged object rows exist; the prior manifest is visible. The client may have no operation response. Reconciliation checks owner heartbeat and reservation expiry; no data is published and reserved capacity remains charged. Required scenario: T8, T22.

### B2 — Upload acknowledged, publication not committed

verified objects and reservation may exist while the inode still names its old manifest. The client has no durable success. Recheck every publication predicate; an abandoned upload is unreachable and retained as a charged orphan. Required scenario: T8.

### B3 — Upload response lost

object state is uncertain; the object may or may not exist. No success is acknowledged. Verify digest/length or repeat the same immutable upload; never publish an unconfirmed object or erase its conservative charge. Required scenario: T10, T22.

### B4 — Publication committed, response lost

new manifest, generation, quotas and receipt committed together. The client sees an ambiguous transport error. Retrieve the receipt under current authorization; do not repeat append or create. Required scenario: T9, T3, T18.

CP13a verifies a control-plane commit surviving response abandonment; CP13b tests connection loss during COMMIT. File publication remains later work.

### B5 — Publication fenced

uploaded objects can exist but the transaction aborts on an epoch, lease, session, handle or reservation predicate. The old manifest remains visible; client receives fenced/stale/conflict. Reconciliation retains orphan charges and an obsolete operation cannot publish later. Required scenario: T12, T13, T19.

CP8/CP11 verify control-plane session fencing; uploads remain later work.

### B6 — Namespace commit, response lost

rename/link/unlink/mkdir result, generations and receipt are one transaction. The caller cannot infer outcome from the connection. Query the receipt; do not replay a rename under another key. Namespace lock prevents ancestor races. Required scenario: T5, T6, T18.

### B7 — Unlink while handles or pins remain

directory entry is gone; zero-link inode and reachable manifests remain. Existing live handles may still access the inode. handle_close or reservation_expire reclaims only after all handles/read/upload pins expire or close, then releases retained logical bytes/inode charge atomically. Physical charge survives until offline deletion. Required scenario: T5, T12.

### B8 — Credential/session revoked during upload

revocation and new epoch are durable while uploaded objects may be pending. Old worker may observe a successful object request but cannot commit. Publication rechecks epochs/revoked_at and marks or reconciles the abandoned upload; issuer revocation cascades through credentials/sessions. Required scenario: T13.

CP9/CP10 verify credential rotation/revocation and session cascade; uploads remain later work.

### B9 — Reservation expires

expired reservation fences publication, but staging/physical charge cannot disappear. The client may be paused. Reconciliation verifies uncertain storage outcomes, transfers accounting conservatively and records deletion_candidates; offline confirmed deletion alone releases physical charge. Required scenario: T22.

### B10 — Workflow takeover

authority row records a new mutation epoch/owner; old sessions may remain persisted but fenced. New holder receives a grant only after authority commit. Old pending operations fail publication; expired workflow sessions require host rebind and cannot silently enter ordinary mode. Required scenario: T19.

### B11 — Receipt expires while outcome ambiguous

receipt_expire (conceptual Stage 1 transaction) locks the authority, credential/session and operation rows in the documented order. At expires_at it atomically changes the expired operation to indeterminate, clears result, retains id/key/request_digest/epochs/committed_at and revokes its client session while incrementing session_epoch. All pending operations from that session are then fenced; session renew/rebind must reject revoked sessions, and only a separately authorized host may mint a replacement. A new session never permits reuse of a retained key. Operation rows are permanent tombstones, including when manifests, objects or reservations still reference them. No expired receipt deletion is permitted in this design.

For admin_operations, receipt_expire likewise retains the credential/key/digest identity permanently and clears only the result payload while setting indeterminate. No session is available or needed for its replay protection: the existing unique credential/key constraint rejects reuse even across credential rotation. Expiry does not revoke administrator access, and current authorization is still required for inspection. getAdminOperation/findAdminOperation can return the tombstone without a workspace ID. Neither table permits an expired key to publish again; mutation retry returns operation_indeterminate, and a client must inspect rather than resubmit an ambiguous action under another key. Administrative receipt tombstones and credential identity rows must not be deleted; future bounded metadata capacity must reject growth rather than discard replay barriers. Required scenario: T18. Structural verification: expired_receipts_retain_identity_and_referenced_state. Runtime expiry/renewal/replay enforcement remains Stage 1 work.

Stage 1a amendment, 2026-10-18 (D19): expiry is lazy on encounter;
`receipts::begin/read_by_id/read_by_key` commit the indeterminate/result-null
transition and preserve identity/key/epochs. CP4/CP12 verify tombstone persistence
and rejected replay. Revoking/incrementing the associated session at receipt
expiry is deferred to the byte-path milestone: acquiring a session update lock
after the receipt would invert the Stage 1a order and can deadlock against
concurrent replays. No sweeper or receipt deletion is implemented.

### B12 — Isolated restore

consistent metadata plus retained objects forms a recovery set; transient sessions, handles, read pins and upload reservations are invalidated. Clients may remember old successes/identities. Restore into an isolated database/prefix, explicitly remap/copy namespace identity, verify every referenced digest/length before writes, fence old identities and preserve recovery-set objects during collection. Required scenario: T15, T21.

### B13 — Truncate shrink then regrow

manifest publication atomically discards extents past EOF and clips the boundary logical_length. Client sees either committed generation, never a mix. Extension/far writes expose zeros for absent extents and invalid tails; inode locking/generation recheck serializes truncate with writes. Required scenario: T23.

## Transaction locks and time

Use READ COMMITTED with explicit row locks. Lock order is schema_meta/deployment_quotas (FOR UPDATE for deployment/reservation transactions) → admin_operations (FOR UPDATE) → workspaces (FOR SHARE for data/namespace mutation; FOR UPDATE for lease, epoch or state changes) → namespace_locks (FOR UPDATE for namespace mutations) → workspace_quotas (FOR UPDATE for every workspace mutation) → credentials (FOR SHARE) → client_sessions (FOR SHARE) → inodes (ascending id, FOR UPDATE) → directory_entries → manifests/objects → upload_reservations → operations (FOR UPDATE) → handles (FOR UPDATE) → read_pins. Rows without a named mode use FOR UPDATE when mutated and FOR SHARE when only validating a publication predicate. Revocation, close, renewal and takeover must acquire conflicting authority locks in the same order.

The Stage 0 model for future file mutations serializes on workspace_quotas; namespace mutations also serialize on namespace_locks. This limits metadata throughput. T1 requires uploads and separate workspaces to remain concurrent; only short metadata transactions serialize. PostgreSQL clock_timestamp() decides expiry immediately before publication; transaction_timestamp() records event times. The pool pins synchronous_commit=on; durable startup/readiness also require reset values and applicable role/database overrides to be on. These checks cannot prove crash-safe hardware or replication policy.

### Stage 1a lock amendment — 2026-10-18

For implemented control-plane jobs the order is workspace FOR SHARE → actor
credential FOR SHARE (FOR UPDATE for self-rotation or a cascade containing the
actor) → credential targets FOR UPDATE top-down → actor session FOR SHARE
(FOR UPDATE when also the target) → receipt SELECT FOR UPDATE / INSERT →
per-target session-mint advisory lock. Namespace/file transaction ordering above
is retained as its Stage 0 baseline and must be reconciled before those routes
are implemented. Control-plane jobs store quota limits but do not acquire
workspace quota locks or charge file bytes.

Receipts follow authentication because they capture verified credential/session
epochs. A same-key waiter cannot hold a share lock the owner later upgrades:
self-targets acquire the strongest actor lock in the prologue, and mint waiters
cannot acquire the advisory lock before their blocked receipt step. This argument
covers only that schedule. Cross-actor contention can deadlock (mutual rotation,
mutual session renew, sibling revocation during parent cascade); PostgreSQL aborts
one transaction with 40P01, mapped to retryable 503. Clients retry with the same
key under current authority; rotation/revocation may require a replacement proof.
No deadlock-freedom claim is made. Persistence tests observe same-key replay and
the mutual rotation/renew schedules.

## Publication fencing

Capture and recheck credential epoch/revoked_at; workspace authorization and mutation epochs; active workflow owner/token/expiry; session epoch/revoked_at/expiry; handle identity, access, closed state and expiry; reservation owner/state/expiry; operation owner_token; inode generation and current manifest. Use authority time and locked rows. Revoking an issuer cascades through issued credentials and sessions in the future transaction. No handler implements these checks in Stage 0.

## Reviewed independent contract decisions

Credential id, kind, workspace_id and issued_by_credential_id are immutable after issuance. Rotation/revocation may change secret/epoch/role/lifetime fields but cannot transfer identity or issuer scope. This closes the reproduced concurrent issuer-rescope/worker-issuance schedule without relying on snapshot reads. Schema verification: credential_scope_cannot_transfer_during_worker_issuance.

receipt_expire owns the state/result expiration transition in both receipt tables and session_epoch/revoked_at changes described in B11. The structural fixture retains a live current manifest and object/reservation references, expires both receipt payloads and demonstrates unique-key rejection. It is not an expiry worker or a claim that future publication fencing is implemented.
