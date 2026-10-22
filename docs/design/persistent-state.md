# Persistent state ownership

Schema contract version 1. [Migrations](../../crates/anchorfs-persistence/migrations/) define the database constraints. Inputs are approved specs 02, 03, 04 and 07 and the independently selected Stage 0 plan. Stage 0 recorded conceptual transactions. Stage 1a implements control-plane transactions, Stage 1b namespace transactions, and Stage 1c byte publication plus logical/staging/physical quota accounting, as annotated below. Stage 2a implements workflow lease routes and session binding. Object reclamation and isolated storage restore remain deferred. Schema coverage alone proves structural constraints; the named executable tests supply behavioral evidence.

Deployment-scope createWorkspace, createCredential, rotateCredential, revokeCredential and createSession use admin_operations receipts. All session-bound workspace mutations use operations. Both bind a request digest and credential epoch; workspace operations additionally bind session and workspace epochs. A receipt permits outcome retrieval, never new authority.

The issuer FK on credentials.id is the documented exception to composite tenant references: a deployment administrator has a NULL workspace. The credentials_issuer_scope trigger enforces a same-workspace non-administrator issuer. All other workspace references include workspace_id. Initial identity grants read/write/admin roles: server traversal/mode checks are Stage 1; only an owner or workspace admin may chmod, only an administrator may chown, and workers cannot hold admin role or mint sessions/leases.

## Column ownership

SQL type/nullability/checks below are structural invariants; semantic invariants are enforced by the annotated implemented owners or remain obligations of the explicitly deferred transactions. Coverage checks enforce an exact table.column set, not behavioral correctness; see the [byte acceptance map](../provenance/stage1c-byte-path-acceptance.md) for executable evidence.

## schema_meta

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| schema_meta.singleton | deployment_initialize / restore | `boolean PRIMARY KEY CHECK (singleton)`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.schema_version | deployment_initialize / restore | `integer NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.object_format_version | deployment_initialize / restore | `integer NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.deployment_id | deployment_initialize / restore | `text NOT NULL CHECK (deployment_id ~ '^[a-z0-9-]{1,64}$')`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |
| schema_meta.chunk_size | deployment_initialize | `integer NOT NULL`; fixed per deployment; startup refuses differing configuration | B12 | compare before serving |
| schema_meta.created_at | deployment_initialize / restore | `timestamptz NOT NULL`; one deployment owns the database and prefix; versions must match binary | B12 | reject identity/version mismatch before serving |

## deployment_quotas

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| deployment_quotas.singleton | deployment_initialize / write_commit / reservation_expire / offline_collect | `boolean PRIMARY KEY CHECK (singleton)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.physical_bytes_limit | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL CHECK (physical_bytes_limit > 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.staging_bytes_limit | deployment_initialize / write_commit / reservation_expire / offline_collect | `bigint NOT NULL CHECK (staging_bytes_limit > 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.physical_bytes_used | deployment_initialize / write_commit / reservation_expire / offline_collect (implemented: meta::initialize, quota::reserve_staging/release_to_orphans; collection deferred) | `bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used >= 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |
| deployment_quotas.staging_bytes_reserved | deployment_initialize / write_commit / reservation_expire / offline_collect (implemented: meta::initialize, quota::reserve_staging/release_to_orphans; collection deferred) | `bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved >= 0)`; physical usage and staging reservations remain separately charged | B1, B4, B9 | retain uncertain charges; adjust only after reconciliation or confirmed offline deletion |

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
| workspaces.id | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `uuid PRIMARY KEY`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.root_inode_id | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `bigint NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.state | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `text NOT NULL CHECK (state IN ('active','maintenance'))`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.authorization_epoch | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `bigint NOT NULL DEFAULT 1`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.mutation_epoch | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `bigint NOT NULL DEFAULT 1`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_owner_credential_id | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `uuid`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_token_hash | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `bytea CHECK (octet_length(workflow_token_hash)=32)`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.workflow_expires_at | lease_acquire / lease_renew / lease_release / lease_takeover (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `timestamptz`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.created_at | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `timestamptz NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |
| workspaces.updated_at | workspace_create / lease_acquire / lease_renew / lease_release / lease_takeover / credential_revoke (implemented: workspaces::create; workflows::acquire/renew/release/takeover) | `timestamptz NOT NULL`; root identity remains in this workspace; lease fields are all present or absent; epochs fence old work | B5, B10, B12 | restore durable authority; advance workflow epoch and require explicit session rebind after takeover |

## workspace_quotas

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| workspace_quotas.workspace_id | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `uuid PRIMARY KEY REFERENCES workspaces(id)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (logical_bytes_limit>0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_inodes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (logical_inodes_limit>0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.physical_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (physical_bytes_limit>0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.staging_bytes_limit | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (limits only)) | `bigint NOT NULL CHECK (staging_bytes_limit>0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_bytes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (initial zero), quota::apply/release_if_due (usage)) | `bigint NOT NULL DEFAULT 0 CHECK (logical_bytes_used>=0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.logical_inodes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create (initial zero), quota::apply/release_if_due (usage)) | `bigint NOT NULL DEFAULT 0 CHECK (logical_inodes_used>=0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.physical_bytes_used | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create, quota::reserve_staging/release_to_orphans; collection deferred) | `bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used>=0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |
| workspace_quotas.staging_bytes_reserved | workspace_create / namespace_mutation / write_commit / reservation_expire / offline_collect (implemented: workspaces::create, quota::reserve_staging/release_to_orphans; collection deferred) | `bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved>=0)`; charge-changing transactions serialize on this row; logical, physical and staging accounting are distinct | B4, B7, B9 | atomic receipt and quota publication; never drop physical charges merely on expiry |

## namespace_locks

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| namespace_locks.workspace_id | workspace_create / namespace_mutation (implemented: workspaces::create (creation only)) | `uuid PRIMARY KEY REFERENCES workspaces(id)`; exclusive namespace lock serializes ancestor validation; generation binds listing cursors | B6 | use committed namespace and receipt; do not replay ambiguous rename under a new key |
| namespace_locks.namespace_generation | workspace_create / namespace_mutation (implemented: workspaces::create, NamespaceGuard::bump) | `bigint NOT NULL DEFAULT 1`; exclusive namespace lock serializes ancestor validation; generation binds listing cursors | B6 | use committed namespace and receipt; do not replay ambiguous rename under a new key |

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
| client_sessions.bound_mutation_epoch | session_create / session_rebind / session_revoke (implemented: sessions::create(bound)/rebind(bound)/renew/revoke; credentials::rotate/revoke) | `bigint`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.expires_at | session_create / session_rebind / session_revoke (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `timestamptz NOT NULL`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |
| client_sessions.revoked_at | session_create / session_rebind / session_revoke / receipt_expire (implemented: sessions::create/renew/rebind/revoke; credentials::rotate/revoke) | `timestamptz`; credential/workspace binding and session epoch cannot silently refresh an ambiguous operation | B5, B8, B10, B11, B12 | invalidate restored transients; host must explicitly rebind a workflow-fenced session |

## inodes

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| inodes.workspace_id | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `uuid NOT NULL REFERENCES workspaces(id)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.id | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `bigint NOT NULL DEFAULT nextval('inodes_id_seq')`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.generation | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `bigint NOT NULL DEFAULT 1`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.kind | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `text NOT NULL CHECK (kind IN ('file','directory','symlink'))`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.size | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `bigint NOT NULL DEFAULT 0 CHECK (size>=0)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.mode | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `integer NOT NULL CHECK (mode BETWEEN 0 AND 511)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.uid | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `bigint NOT NULL CHECK (uid BETWEEN 0 AND 4294967295)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.gid | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `bigint NOT NULL CHECK (gid BETWEEN 0 AND 4294967295)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.atime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::* (including explicit times in namespace::set_attributes), handles::* for metadata; byte/expiry transactions remain future) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.mtime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::* (including explicit times in namespace::set_attributes), handles::* for metadata; byte/expiry transactions remain future) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.ctime | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `timestamptz NOT NULL`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.nlink | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `integer NOT NULL DEFAULT 1 CHECK (nlink>=0)`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.current_manifest_id | write_commit / truncate_commit (future byte transactions) | `uuid`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.symlink_target | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `text`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.unlinked_at | workspace_create / namespace_mutation / write_commit / truncate_commit / handle_close / reservation_expire (implemented: workspaces::create, namespace::*, handles::* for metadata; byte/expiry transactions remain future) | `timestamptz`; stable inode identity; generation increases on mutation; manifests belong to this inode; zero links may remain pinned | B4, B7, B13 | use committed generation and manifest; reclaim only after every handle and read/upload pin is gone |
| inodes.charge_released_at | namespace_mutation / handle_close (implemented: quota::release_if_due) | `timestamptz NULL`; set once only for nlink=0 with no open unexpired handle | B7 | charge retained until eligible unlink/replacement/close releases it atomically |

## directory_entries

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| directory_entries.workspace_id | namespace_mutation  (implemented: namespace::* via NamespaceGuard)| `uuid NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.parent_inode_id | namespace_mutation  (implemented: namespace::* via NamespaceGuard)| `bigint NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.name | namespace_mutation  (implemented: namespace::* via NamespaceGuard)| `text COLLATE "C" NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255 AND position('/' in name)=0 AND name NOT IN ('.','..'))`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |
| directory_entries.inode_id | namespace_mutation  (implemented: namespace::* via NamespaceGuard)| `bigint NOT NULL`; unique byte-ordered name under a parent in the same workspace; cycle/type checks in future transaction | B6, B7 | atomic namespace receipt resolves outcome; retain unlinked inode while pinned |

## upload_reservations

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| upload_reservations.workspace_id | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `uuid NOT NULL REFERENCES workspaces(id)`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.id | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.operation_id | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.credential_id | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `uuid NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.reserved_bytes | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `bigint NOT NULL CHECK (reserved_bytes>0)`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.state | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `text NOT NULL CHECK (state IN ('active','consumed','expired','failed'))`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.heartbeat_at | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `timestamptz NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |
| upload_reservations.expires_at | write_commit / reservation_expire / reconcile_uncertain (implemented: reservations::insert/charge/consume/fail/fence_overdue) | `timestamptz NOT NULL`; reservation and owner operation persist before upload; expiry fences publication without freeing physical charge | B1, B3, B5, B9 | reconcile uncertain upload; move retained charge to orphan accounting without making data visible |

## objects

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| objects.workspace_id | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `uuid NOT NULL REFERENCES workspaces(id)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.id | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `uuid NOT NULL`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.byte_length | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `bigint NOT NULL CHECK (byte_length>0)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.sha256 | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `bytea NOT NULL CHECK (octet_length(sha256)=32)`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.state | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `text NOT NULL CHECK (state IN ('staged','uncertain','published'))`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.reservation_id | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `uuid NOT NULL`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.uploaded_at | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `timestamptz`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |
| objects.published_at | write_commit / reconcile_uncertain / offline_collect (implemented: objects::stage/mark_uploaded/publish/orphan) | `timestamptz`; unique opaque identity and expected digest; staged before upload; publish only after verified persistence | B1, B2, B3, B4, B9 | verify or repeat identical immutable upload; unreachable objects become deletion candidates and stay charged |

## manifests

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| manifests.workspace_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.inode_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `bigint NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.size | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `bigint NOT NULL CHECK (size>=0)`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.chunk_size | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `integer NOT NULL CHECK (chunk_size>0)`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.created_by_operation_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |
| manifests.generation | write_commit / truncate_commit / replace_commit / handle_open (implemented: manifests::publish_empty/write/truncate/replace) | `bigint NOT NULL CHECK (generation >= 1)`; inode generation at publication; <= inodes.generation | B4, B13 | immutable generation of content |
| manifests.created_at | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `timestamptz NOT NULL`; immutable published description belongs to one inode; every referenced object persisted before commit | B2, B4, B13 | serve only complete committed manifest; preserve recovery-set reachability before offline collection |

## manifest_chunks

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| manifest_chunks.workspace_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.manifest_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.chunk_index | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `integer NOT NULL CHECK (chunk_index>=0)`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.chunk_size | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `integer NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.object_id | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `uuid NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.sha256 | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `bytea NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |
| manifest_chunks.logical_length | write_commit / truncate_commit / offline_collect (implemented: manifests::publish_empty/write/truncate/replace) | `integer NOT NULL`; immutable chunk extent; digest FK matches object; chunk index times chunk size must be less than manifest size | B4, B13 | absent indexes and bytes beyond logical_length read as zero; never expose discarded old tails |

## operations

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| operations.workspace_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.credential_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.client_session_id | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `uuid NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.session_epoch | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.credential_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.authorization_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.mutation_epoch_at_start | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bigint NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.owner_token | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bytea NOT NULL CHECK (octet_length(owner_token)=32)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.heartbeat_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `timestamptz NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.idempotency_key | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 256)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.op_type | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `text NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.request_digest | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `bytea NOT NULL CHECK (octet_length(request_digest)=32)`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.expiry_fences_session | receipt_expire (implemented: receipts::tombstone) | `boolean NOT NULL DEFAULT false`; true only for tombstones of pending/committed receipts; durable B11 follow-up eligibility | B11 | retry session fencing after process death; failed receipts excluded |
| operations.state | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain / receipt_expire (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `text NOT NULL CHECK (state IN ('pending','committed','failed','indeterminate'))`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.result | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain / receipt_expire (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `jsonb`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.created_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `timestamptz`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.committed_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `timestamptz`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |
| operations.expires_at | namespace_mutation / write_commit / truncate_commit / reconcile_uncertain (implemented: receipts::peek/begin/lock_pending/commit_result/fail_result/fence_expired_pending/read_by_id/read_by_key) | `timestamptz NOT NULL`; unique workspace/credential/key binds digest, owner, epochs and receipt | B1, B4, B5, B6, B11 | resolve durable receipt first; heartbeat takeover serializes and cannot duplicate publication |

## handles

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| handles.workspace_id | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.id | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.inode_id | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `bigint NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.client_session_id | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.credential_id | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `uuid NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.access_read | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `boolean NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.access_write | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `boolean NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.opened_at | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `timestamptz NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.expires_at | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `timestamptz NOT NULL`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |
| handles.closed_at | handle_open / handle_renew / handle_close (implemented: handles::open_file/renew/close); expiry sweeper future | `timestamptz`; inode identity, granted access and client-session/credential bindings persist; close and expiry fence publication | B5, B7, B12 | never reopen expired identity implicitly; hold unlinked data until every pin is released |

## read_pins

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| read_pins.workspace_id | pin_create / reservation_expire / restore (implemented: pins::create/load_for_read/release) | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.id | pin_create / reservation_expire / restore (implemented: pins::create/load_for_read/release) | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.manifest_id | pin_create / reservation_expire / restore (implemented: pins::create/load_for_read/release) | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.client_session_id | pin_create / reservation_expire / restore (implemented: pins::create/load_for_read/release) | `uuid NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |
| read_pins.expires_at | pin_create / reservation_expire / restore (implemented: pins::create/load_for_read/release) | `timestamptz NOT NULL`; bounded client-session pin protects exactly one immutable manifest | B7, B12 | release expired pins only under authority locks; invalidate transient pins after restore |

## deletion_candidates

| Column | Owner transaction (conceptual, Stage 1+) | Invariant | Crash boundary | Recovery rule |
|---|---|---|---|---|
| deletion_candidates.workspace_id | reconcile_uncertain / offline_collect (implemented: objects::orphan, receipts::fence_expired_pending; collection deferred) | `uuid NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.object_id | reconcile_uncertain / offline_collect (implemented: objects::orphan, receipts::fence_expired_pending; collection deferred) | `uuid NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.reason | reconcile_uncertain / offline_collect (implemented: objects::orphan, receipts::fence_expired_pending; collection deferred) | `text NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.physical_bytes | reconcile_uncertain / offline_collect (implemented: objects::orphan, receipts::fence_expired_pending; collection deferred) | `bigint NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |
| deletion_candidates.recorded_at | reconcile_uncertain / offline_collect (implemented: objects::orphan, receipts::fence_expired_pending; collection deferred) | `timestamptz NOT NULL`; single orphan record; bytes remain physically charged until deletion confirmed; automatic deletion disabled | B3, B9, B12 | offline collector freezes all writers and proves live/pinned/recovery reachability before deletion |

## Crash boundaries

The baseline descriptions below state intended recovery boundaries. Dated
verification notes identify the implemented/tested portions; collection and
isolated restore remain later work.

### B1 — Reservation committed, upload not started

operation, reservation and staged object rows exist; the prior manifest is visible. The client may have no operation response. Reconciliation checks owner heartbeat and reservation expiry; no data is published and reserved capacity remains charged. Required scenario: T8, T22.

Stage 1c verifies staged admission and old-manifest visibility with BY4, and
reservation/charge recovery before publication with BY5/BY5b/BY5c/BY5d.

### B2 — Upload acknowledged, publication not committed

verified objects and reservation may exist while the inode still names its old manifest. The client has no durable success. Recheck every publication predicate; an abandoned upload is unreachable and retained as a charged orphan. Required scenario: T8.

Stage 1c verifies charged unpublished objects and durable expiry fencing with
BY5/BY5b/BY5c/BY5d and native old-generation reads.

### B3 — Upload response lost

object state is uncertain; the object may or may not exist. No success is acknowledged. Verify digest/length or repeat the same immutable upload; never publish an unconfirmed object or erase its conservative charge. Required scenario: T10, T22.

Stage 1c verifies BY6 provider request/reply loss and integrity failures, with
raw object observations and conservative orphan charges.

### B4 — Publication committed, response lost

new manifest, generation, quotas and receipt committed together. The client sees an ambiguous transport error. Retrieve the receipt under current authorization; do not repeat append or create. Required scenario: T9, T3, T18.

CP13a verifies a control-plane commit surviving response abandonment; CP13b tests connection loss during COMMIT. Stage 1c verifies file publication and lost responses with BY3/BY3c/BY3d/BY19, including actual connection termination during COMMIT.

### B5 — Publication fenced

uploaded objects can exist but the transaction aborts on an epoch, lease, session, handle or reservation predicate. The old manifest remains visible; client receives fenced/stale/conflict. Reconciliation retains orphan charges and an obsolete operation cannot publish later. Required scenario: T12, T13, T19.

CP8/CP11 verify control-plane session fencing. Stage 1c verifies upload publication fences with BY8/BY9, including reservation expiry and fixture workflow leases.

### B6 — Namespace commit, response lost

rename/link/unlink/mkdir result, generations and receipt are one transaction. The caller cannot infer outcome from the connection. Query the receipt; do not replay a rename under another key. Namespace lock prevents ancestor races. Required scenario: T5, T6, T18.

Stage 1b verifies B6 through NS2 receipt replay/mismatch, NS3 lost-response recovery and NS3b expired-receipt rejection.

### B7 — Unlink while handles or pins remain

directory entry is gone; zero-link inode and reachable manifests remain. Existing live handles may still access the inode. handle_close or reservation_expire reclaims only after all handles/read/upload pins expire or close, then releases retained logical bytes/inode charge atomically. Physical charge survives until offline deletion. Required scenario: T5, T12.

Stage 1b verifies metadata retention through NS6/NS6b and the namespace_tx handle release/renewal schedules. Stage 1c verifies manifests/pins and unlink-open byte publication with BY10/BY10b/BY13; sweepers and physical collection remain future work.

### B8 — Credential/session revoked during upload

revocation and new epoch are durable while uploaded objects may be pending. Old worker may observe a successful object request but cannot commit. Publication rechecks epochs/revoked_at and marks or reconciles the abandoned upload; issuer revocation cascades through credentials/sessions. Required scenario: T13.

CP9/CP10 verify credential rotation/revocation and session cascade. Stage 1c verifies rejection after upload with BY9 credential revocation/rotation and epoch schedules.

### B9 — Reservation expires

expired reservation fences publication, but staging/physical charge cannot disappear. The client may be paused. Reconciliation verifies uncertain storage outcomes, transfers accounting conservatively and records deletion_candidates; offline confirmed deletion alone releases physical charge. Required scenario: T22.

Stage 1c verifies expiry fencing, failed receipts and physical/staging transfers
with BY5/BY5b/BY5d/BY7/BY9 and the deployment/bucket oracles.

### B10 — Workflow takeover

authority row records a new mutation epoch/owner; old sessions may remain persisted but fenced. New holder receives a grant only after authority commit. Old pending operations fail publication; expired workflow sessions require host rebind and cannot silently enter ordinary mode. Required scenario: T19.

Stage 2a amendment (2026-10-20): `takeover_fences_old_lease_upload_and_successor_publishes`
(WL-B2) acquires real grants through HTTP, parks a bound upload, expires and
takes over the lease, then proves the old publication and reads are fenced,
the orphan is charged, and a successor publishes through its own session.

Stage 2a lock amendment: lease transactions perform unlocked credential
preauthentication (kind, role, workspace) then workspace FOR UPDATE, credential
FOR SHARE, actor session FOR SHARE and receipt dispatch. Bind/rebind retain
workspace FOR SHARE, serializing with the transitions. Deadline-bounded lock
waits prevent abandoned waiters from later acquiring grants. Child insert FK
locks and receipt fence/peek paths remain subject to the workspace-first order;
review must assess their implicit key-share locks as well.

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

Stage 1c amendment, 2026-10-20 (W35): the deferred session fence is implemented.
The tombstone transaction first fences expired pending reservations and clears
the receipt result. A separate idempotent transaction locks the session FOR
UPDATE, verifies the receipt's indeterminate state and durable
expiry_fences_session flag, then revokes the session and increments its epoch
once. Every subsequent encounter repeats eligibility and retries the follow-up.
This supersedes the conceptual single-transaction revocation above; it preserves
session-before-receipt ordering. Failed receipts persist false eligibility when
tombstoned and never revoke their session. Administrative receipts have no
session. Verified by expired_receipt_tombstone_then_session_fence_once and the
HTTP crash/follow-up test; no sweeper is introduced.

### B12 — Isolated restore

consistent metadata plus retained objects forms a recovery set; transient sessions, handles, read pins and upload reservations are invalidated. Clients may remember old successes/identities. Restore into an isolated database/prefix, explicitly remap/copy namespace identity, verify every referenced digest/length before writes, fence old identities and preserve recovery-set objects during collection. Required scenario: T15, T21.

### B13 — Truncate shrink then regrow

manifest publication atomically discards extents past EOF and clips the boundary logical_length. Client sees either committed generation, never a mix. Extension/far writes expose zeros for absent extents and invalid tails; inode locking/generation recheck serializes truncate with writes. Required scenario: T23.

Stage 1c verifies clipped-prefix reads, zero fill and retry after a racing
truncate with BY10/BY11/BY8 and the sequential byte model.

## Transaction locks and time

Use READ COMMITTED with explicit row locks. Lock order is schema_meta/deployment_quotas (FOR UPDATE for deployment/reservation transactions) → admin_operations (FOR UPDATE) → workspaces (FOR SHARE for data/namespace mutation; FOR UPDATE for lease, epoch or state changes) → namespace_locks (FOR UPDATE for namespace mutations) → workspace_quotas (FOR UPDATE for every workspace mutation) → credentials (FOR SHARE) → client_sessions (FOR SHARE) → inodes (ascending id, FOR UPDATE) → directory_entries → manifests/objects → upload_reservations → operations (FOR UPDATE) → handles (FOR UPDATE) → read_pins. Rows without a named mode use FOR UPDATE when mutated and FOR SHARE when only validating a publication predicate. Revocation, close, renewal and takeover must acquire conflicting authority locks in the same order.

The Stage 0 model for future file mutations serializes on workspace_quotas; namespace mutations also serialize on namespace_locks. This limits metadata throughput. T1 requires uploads and separate workspaces to remain concurrent; only short metadata transactions serialize. PostgreSQL clock_timestamp() decides expiry immediately before publication; transaction_timestamp() records event times. The pool pins synchronous_commit=on; durable startup/readiness also require reset values and applicable role/database overrides to be on. These checks cannot prove crash-safe hardware or replication policy.

### Stage 1a lock amendment — 2026-10-18

For implemented control-plane jobs the order is workspace FOR SHARE → actor
credential FOR SHARE (FOR UPDATE for self-rotation or a cascade containing the
actor) → actor session FOR SHARE (FOR UPDATE when also the target) → receipt
SELECT FOR UPDATE / INSERT → mutation target credentials FOR UPDATE top-down
or target session FOR UPDATE → per-target session-admission advisory lock.
The admission lock applies to mint and rebind; expired rebind consumes a slot,
while active rebind retains its slot. Session revocation updates follow the
locked credential cascade. This review clarification records actual target locks
after receipts; self-target actor locks remain in the prologue to prevent the
same-key lock-upgrade cycle. Namespace/file transaction ordering above
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
all three deadlock schedules. A final database-clock check after all publication
waits enforces the original actor credential/session expiry; row locks alone
do not preserve a lifetime. Fresh mutation/receipt writes roll back on expiry.

## Publication fencing

Capture and recheck credential epoch/revoked_at; workspace authorization and mutation epochs; active workflow owner/token/expiry; session epoch/revoked_at/expiry; handle identity, access, closed state and expiry; reservation owner/state/expiry; operation owner_token; inode generation and current manifest. Use authority time and locked rows. Revoking an issuer cascades through issued credentials and sessions in the future transaction. No handler implements these checks in Stage 0.

## Reviewed independent contract decisions

Credential id, kind, workspace_id and issued_by_credential_id are immutable after issuance. Rotation/revocation may change secret/epoch/role/lifetime fields but cannot transfer identity or issuer scope. This closes the reproduced concurrent issuer-rescope/worker-issuance schedule without relying on snapshot reads. Schema verification: credential_scope_cannot_transfer_during_worker_issuance.

receipt_expire owns the state/result expiration transition in both receipt tables and session_epoch/revoked_at changes described in B11. The structural fixture retains a live current manifest and object/reservation references, expires both receipt payloads and demonstrates unique-key rejection. It is not an expiry worker or a claim that future publication fencing is implemented.

Stage 1a admission recheck (2026-10-18): renew also holds the same per-credential
admission lock through COMMIT. Without this, another transaction could count an
old committed expiry after it lapses while a renewal's extension is uncommitted,
admitting a replacement before the renewal becomes visible. Target expiry is
checked after acquiring admission, so renew still fences an expired target;
rebind remains the explicit reactivation path. The lock covers create, renew and
rebind, and a blocked-minter regression verifies the MVCC interval.

## Stage 1b namespace lock amendment — 2026-10-18

Namespace and handle transactions use this relative order: workspace FOR SHARE → actor credential FOR SHARE → actor session FOR SHARE → operations receipt → namespace_locks FOR UPDATE (entry mutations) → all affected inodes FOR UPDATE in a single ascending-id statement → directory_entries → workspace_quotas FOR UPDATE (charge changes) → handles FOR UPDATE → session-keyed advisory lock 7002 (open/renew admission). This resolves the Stage 1a reconciliation requirement and supersedes the Stage 0 quota-before-inode order for namespace and future byte transactions; the baseline text above remains historical.

Reads and open-without-create do not take the namespace lock. Reads use committed, non-snapshot path walks. The inode lock serializes handle renewal against charge release. A persisted charge_released_at marker makes release idempotent and accounting independently checkable; the root inode is uncharged.

## Stage 1c byte lock amendment — 2026-10-20

The byte publication order is workspace FOR SHARE → actor credential FOR SHARE
→ actor session FOR SHARE → operations FOR UPDATE → namespace_locks FOR UPDATE
(entry operations only) → inodes FOR UPDATE in ascending order → handles (plain
validation read; FOR UPDATE in close/renew) → upload_reservations FOR UPDATE
→ manifests/chunks/objects/deletion_candidates → workspace_quotas FOR UPDATE
→ deployment_quotas FOR UPDATE → read_pins → session-keyed advisory locks
(7002 handles, 7003 pins). Admission inserts private reservation/object rows
before charging quotas. Retry swaps release the old reservation before charging
the new one within a savepoint; quota rows may be touched twice. Every
transaction taking two or more of operations, reservations FOR UPDATE and quota
rows locks the operation first. Replace's per-chunk stage transaction takes
only its reservation FOR SHARE and inserts its private object row. Lazy fences
act on operation identity without actor/session locks. B11 follow-ups take the
session FOR UPDATE and then read their operation. No backend I/O may run in an
open database transaction.

Stage 2a correction (2026-10-20): replacement's per-chunk staging transaction
now takes workspace FOR SHARE before reservation FOR SHARE and object INSERT.
The existing workspace lock prevents INSERT's implicit workspace FK check from
introducing a reservation-to-workspace wait. Body reads and backend I/O remain
outside this short metadata transaction.

The Stage 1b quota-before-handles wording above is corrected to the implemented
inode → handle → quota order used by close. The Stage 0 deployment-quota-first
placement is superseded. Namespace operations otherwise retain their relative
order. The persistence primitives and handle blocking schedules are tested;
service-level schedules verify overlapping writes, append retry and quota races
without holding transactions across backend I/O. BY20 verifies the eight-task randomized workload against the database deadlock
counter and committed-history model.
