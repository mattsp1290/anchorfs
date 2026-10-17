CREATE TABLE credentials (
 id uuid PRIMARY KEY, workspace_id uuid REFERENCES workspaces(id), kind text NOT NULL CHECK (kind IN ('administrator','host','worker')), role text NOT NULL CHECK (role IN ('read','write','admin')),
 issued_by_credential_id uuid REFERENCES credentials(id), uid bigint NOT NULL CHECK (uid BETWEEN 0 AND 4294967295), gid bigint NOT NULL CHECK (gid BETWEEN 0 AND 4294967295), secret_hash bytea NOT NULL CHECK (octet_length(secret_hash)=32), epoch bigint NOT NULL DEFAULT 1,
 max_reservation_bytes bigint NOT NULL CHECK (max_reservation_bytes>0), expires_at timestamptz, revoked_at timestamptz, created_at timestamptz NOT NULL,
 CONSTRAINT credentials_scope_check CHECK (kind='administrator' OR workspace_id IS NOT NULL),
 CONSTRAINT credentials_administrator_scope_check CHECK (kind<>'administrator' OR (role='admin' AND workspace_id IS NULL)),
 CONSTRAINT credentials_worker_role_check CHECK (kind<>'worker' OR role<>'admin'), UNIQUE (workspace_id,id)
);
CREATE FUNCTION check_credentials_issuer_scope() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS (SELECT 1 FROM credentials WHERE id=NEW.issued_by_credential_id AND workspace_id IS NOT NULL AND workspace_id IS DISTINCT FROM NEW.workspace_id) THEN
 RAISE EXCEPTION 'issuer belongs to another workspace' USING ERRCODE='23514', CONSTRAINT='credentials_issuer_scope';
 END IF;
 -- Also prevent an issuer workspace change from invalidating existing issued credentials.
 IF TG_OP='UPDATE' AND NEW.workspace_id IS NOT NULL AND EXISTS (SELECT 1 FROM credentials WHERE issued_by_credential_id=NEW.id AND workspace_id IS DISTINCT FROM NEW.workspace_id) THEN
 RAISE EXCEPTION 'issuer workspace conflicts with issued credentials' USING ERRCODE='23514', CONSTRAINT='credentials_issuer_scope';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER credentials_issuer_scope BEFORE INSERT OR UPDATE ON credentials FOR EACH ROW EXECUTE FUNCTION check_credentials_issuer_scope();
CREATE TABLE client_sessions (
 id uuid PRIMARY KEY, workspace_id uuid NOT NULL REFERENCES workspaces(id), credential_id uuid NOT NULL, credential_epoch bigint NOT NULL, session_epoch bigint NOT NULL, bound_mutation_epoch bigint, expires_at timestamptz NOT NULL, revoked_at timestamptz,
 UNIQUE (workspace_id,id), CONSTRAINT client_sessions_credential_fk FOREIGN KEY (workspace_id,credential_id) REFERENCES credentials(workspace_id,id)
);
ALTER TABLE workspaces ADD CONSTRAINT workspaces_workflow_owner_fk FOREIGN KEY (id,workflow_owner_credential_id) REFERENCES credentials(workspace_id,id);
ALTER TABLE admin_operations ADD CONSTRAINT admin_operations_credential_fk FOREIGN KEY (credential_id) REFERENCES credentials(id);
