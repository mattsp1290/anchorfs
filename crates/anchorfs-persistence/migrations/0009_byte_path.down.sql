DROP INDEX read_pins_session_idx;
DROP INDEX upload_reservations_active_expiry_idx;
DROP INDEX upload_reservations_operation_idx;
DROP INDEX objects_reservation_idx;
ALTER TABLE schema_meta DROP COLUMN chunk_size;
ALTER TABLE manifests DROP CONSTRAINT manifests_generation_positive;
ALTER TABLE manifests DROP COLUMN generation;
