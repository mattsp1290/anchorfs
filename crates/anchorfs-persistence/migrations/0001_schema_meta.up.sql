CREATE TABLE schema_meta (
 singleton boolean PRIMARY KEY CHECK (singleton), schema_version integer NOT NULL,
 object_format_version integer NOT NULL, deployment_id text NOT NULL CHECK (deployment_id ~ '^[a-z0-9-]{1,64}$'), created_at timestamptz NOT NULL
);
CREATE TABLE deployment_quotas (
 singleton boolean PRIMARY KEY CHECK (singleton), physical_bytes_limit bigint NOT NULL CHECK (physical_bytes_limit > 0), staging_bytes_limit bigint NOT NULL CHECK (staging_bytes_limit > 0), physical_bytes_used bigint NOT NULL DEFAULT 0 CHECK (physical_bytes_used >= 0), staging_bytes_reserved bigint NOT NULL DEFAULT 0 CHECK (staging_bytes_reserved >= 0)
);
CREATE TABLE admin_operations (
 id uuid PRIMARY KEY, credential_id uuid NOT NULL, credential_epoch_at_start bigint NOT NULL,
 idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 256),
 op_type text NOT NULL CHECK (op_type IN ('workspace_create','credential_issue','credential_rotate','credential_revoke','session_create')),
 request_digest bytea NOT NULL CHECK (octet_length(request_digest)=32), owner_token bytea NOT NULL CHECK (octet_length(owner_token)=32), heartbeat_at timestamptz NOT NULL,
 state text NOT NULL CHECK (state IN ('pending','committed','failed','indeterminate')), result jsonb, created_at timestamptz, committed_at timestamptz, expires_at timestamptz NOT NULL,
 UNIQUE (credential_id,idempotency_key)
);
