CREATE TABLE workspaces (
 id uuid PRIMARY KEY, root_inode_id bigint NOT NULL, state text NOT NULL CHECK (state IN ('active','maintenance')),
 authorization_epoch bigint NOT NULL DEFAULT 1, mutation_epoch bigint NOT NULL DEFAULT 1,
 workflow_owner_credential_id uuid, workflow_token_hash bytea CHECK (octet_length(workflow_token_hash)=32), workflow_expires_at timestamptz,
 created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
 CONSTRAINT workspaces_workflow_all_or_none CHECK ((workflow_owner_credential_id IS NULL AND workflow_token_hash IS NULL AND workflow_expires_at IS NULL) OR (workflow_owner_credential_id IS NOT NULL AND workflow_token_hash IS NOT NULL AND workflow_expires_at IS NOT NULL))
);
CREATE TABLE workspace_quotas (
 workspace_id uuid PRIMARY KEY REFERENCES workspaces(id),
 logical_bytes_limit bigint NOT NULL CHECK (logical_bytes_limit>0), logical_inodes_limit bigint NOT NULL CHECK (logical_inodes_limit>0), physical_bytes_limit bigint NOT NULL CHECK (physical_bytes_limit>0), staging_bytes_limit bigint NOT NULL CHECK (staging_bytes_limit>0),
 logical_bytes_used bigint NOT NULL DEFAULT 0 CHECK (logical_bytes_used>=0), logical_inodes_used bigint NOT NULL DEFAULT 0 CHECK (logical_inodes_used>=0), physical_bytes_used bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used>=0), staging_bytes_reserved bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved>=0)
);
CREATE TABLE namespace_locks (workspace_id uuid PRIMARY KEY REFERENCES workspaces(id), namespace_generation bigint NOT NULL DEFAULT 1);
