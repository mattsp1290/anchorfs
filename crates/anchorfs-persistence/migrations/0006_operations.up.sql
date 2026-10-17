CREATE TABLE operations (
 workspace_id uuid NOT NULL, id uuid NOT NULL, credential_id uuid NOT NULL, client_session_id uuid NOT NULL, session_epoch bigint NOT NULL, credential_epoch_at_start bigint NOT NULL, authorization_epoch_at_start bigint NOT NULL, mutation_epoch_at_start bigint NOT NULL,
 owner_token bytea NOT NULL CHECK (octet_length(owner_token)=32), heartbeat_at timestamptz NOT NULL, idempotency_key text NOT NULL CHECK (octet_length(idempotency_key) BETWEEN 1 AND 256), op_type text NOT NULL, request_digest bytea NOT NULL CHECK (octet_length(request_digest)=32), state text NOT NULL CHECK (state IN ('pending','committed','failed','indeterminate')), result jsonb, created_at timestamptz, committed_at timestamptz, expires_at timestamptz NOT NULL,
 PRIMARY KEY (workspace_id,id), CONSTRAINT operations_receipt_key UNIQUE (workspace_id,credential_id,idempotency_key),
 CONSTRAINT operations_credential_fk FOREIGN KEY (workspace_id,credential_id) REFERENCES credentials(workspace_id,id), CONSTRAINT operations_session_fk FOREIGN KEY (workspace_id,client_session_id) REFERENCES client_sessions(workspace_id,id)
);
CREATE TABLE handles (
 workspace_id uuid NOT NULL, id uuid NOT NULL, inode_id bigint NOT NULL, client_session_id uuid NOT NULL, credential_id uuid NOT NULL, access_read boolean NOT NULL, access_write boolean NOT NULL, opened_at timestamptz NOT NULL, expires_at timestamptz NOT NULL, closed_at timestamptz,
 PRIMARY KEY (workspace_id,id), CHECK (access_read OR access_write),
 CONSTRAINT handles_inode_fk FOREIGN KEY (workspace_id,inode_id) REFERENCES inodes(workspace_id,id), CONSTRAINT handles_session_fk FOREIGN KEY (workspace_id,client_session_id) REFERENCES client_sessions(workspace_id,id), CONSTRAINT handles_credential_fk FOREIGN KEY (workspace_id,credential_id) REFERENCES credentials(workspace_id,id)
);
CREATE TABLE read_pins (
 workspace_id uuid NOT NULL, id uuid NOT NULL, manifest_id uuid NOT NULL, client_session_id uuid NOT NULL, expires_at timestamptz NOT NULL,
 PRIMARY KEY (workspace_id,id), CONSTRAINT read_pins_manifest_fk FOREIGN KEY (workspace_id,manifest_id) REFERENCES manifests(workspace_id,id), CONSTRAINT read_pins_session_fk FOREIGN KEY (workspace_id,client_session_id) REFERENCES client_sessions(workspace_id,id)
);
CREATE TABLE deletion_candidates (
 workspace_id uuid NOT NULL, object_id uuid NOT NULL, reason text NOT NULL, physical_bytes bigint NOT NULL, recorded_at timestamptz NOT NULL,
 PRIMARY KEY (workspace_id,object_id), CONSTRAINT deletion_candidates_object_fk FOREIGN KEY (workspace_id,object_id) REFERENCES objects(workspace_id,id)
);
ALTER TABLE upload_reservations ADD CONSTRAINT upload_reservations_operation_fk FOREIGN KEY (workspace_id,operation_id) REFERENCES operations(workspace_id,id);
ALTER TABLE objects ADD CONSTRAINT objects_reservation_fk FOREIGN KEY (workspace_id,reservation_id) REFERENCES upload_reservations(workspace_id,id);
ALTER TABLE manifests ADD CONSTRAINT manifests_operation_fk FOREIGN KEY (workspace_id,created_by_operation_id) REFERENCES operations(workspace_id,id);
