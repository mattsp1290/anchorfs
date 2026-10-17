CREATE TABLE upload_reservations (
 workspace_id uuid NOT NULL REFERENCES workspaces(id), id uuid NOT NULL, operation_id uuid NOT NULL, credential_id uuid NOT NULL, reserved_bytes bigint NOT NULL CHECK (reserved_bytes>0), state text NOT NULL CHECK (state IN ('active','consumed','expired','failed')), heartbeat_at timestamptz NOT NULL, expires_at timestamptz NOT NULL,
 PRIMARY KEY (workspace_id,id), CONSTRAINT upload_reservations_credential_fk FOREIGN KEY (workspace_id,credential_id) REFERENCES credentials(workspace_id,id)
);
CREATE TABLE objects (
 workspace_id uuid NOT NULL REFERENCES workspaces(id), id uuid NOT NULL, byte_length bigint NOT NULL CHECK (byte_length>0), sha256 bytea NOT NULL CHECK (octet_length(sha256)=32), state text NOT NULL CHECK (state IN ('staged','uncertain','published')), reservation_id uuid NOT NULL, uploaded_at timestamptz, published_at timestamptz,
 PRIMARY KEY (workspace_id,id), UNIQUE (workspace_id,id,sha256)
);
CREATE TABLE manifests (
 workspace_id uuid NOT NULL, id uuid NOT NULL, inode_id bigint NOT NULL, size bigint NOT NULL CHECK (size>=0), chunk_size integer NOT NULL CHECK (chunk_size>0), created_by_operation_id uuid NOT NULL, created_at timestamptz NOT NULL,
 PRIMARY KEY (workspace_id,id), UNIQUE (workspace_id,inode_id,id), UNIQUE (workspace_id,id,chunk_size), CONSTRAINT manifests_inode_fk FOREIGN KEY (workspace_id,inode_id) REFERENCES inodes(workspace_id,id)
);
CREATE TABLE manifest_chunks (
 workspace_id uuid NOT NULL, manifest_id uuid NOT NULL, chunk_index integer NOT NULL CHECK (chunk_index>=0), chunk_size integer NOT NULL, object_id uuid NOT NULL, sha256 bytea NOT NULL, logical_length integer NOT NULL,
 PRIMARY KEY (workspace_id,manifest_id,chunk_index), CONSTRAINT manifest_chunks_logical_length_check CHECK (logical_length>0 AND logical_length<=chunk_size),
 CONSTRAINT manifest_chunks_manifest_fk FOREIGN KEY (workspace_id,manifest_id,chunk_size) REFERENCES manifests(workspace_id,id,chunk_size),
 CONSTRAINT manifest_chunks_object_fk FOREIGN KEY (workspace_id,object_id,sha256) REFERENCES objects(workspace_id,id,sha256)
);
ALTER TABLE inodes ADD CONSTRAINT inodes_current_manifest_fk FOREIGN KEY (workspace_id,id,current_manifest_id) REFERENCES manifests(workspace_id,inode_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE FUNCTION reject_immutable_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'immutable manifest data' USING ERRCODE='AF001', CONSTRAINT=TG_NAME; END $$;
CREATE TRIGGER manifests_immutable BEFORE UPDATE ON manifests FOR EACH ROW EXECUTE FUNCTION reject_immutable_update();
CREATE TRIGGER manifest_chunks_immutable BEFORE UPDATE ON manifest_chunks FOR EACH ROW EXECUTE FUNCTION reject_immutable_update();
