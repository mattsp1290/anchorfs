ALTER TABLE manifests ADD COLUMN generation bigint NOT NULL DEFAULT 1;
ALTER TABLE manifests ALTER COLUMN generation DROP DEFAULT;
ALTER TABLE manifests ADD CONSTRAINT manifests_generation_positive CHECK (generation >= 1);
ALTER TABLE schema_meta ADD COLUMN chunk_size integer NOT NULL DEFAULT 4194304 CHECK (chunk_size BETWEEN 4096 AND 4194304);
ALTER TABLE schema_meta ALTER COLUMN chunk_size DROP DEFAULT;
CREATE INDEX objects_reservation_idx ON objects(workspace_id, reservation_id);
CREATE INDEX upload_reservations_operation_idx ON upload_reservations(workspace_id, operation_id);
CREATE INDEX upload_reservations_active_expiry_idx ON upload_reservations(workspace_id, expires_at) WHERE state = 'active';
CREATE INDEX read_pins_session_idx ON read_pins(workspace_id, client_session_id);
-- Retain the failed-receipt exclusion after result is cleared at tombstoning.
ALTER TABLE operations ADD COLUMN expiry_fences_session boolean NOT NULL DEFAULT false;
