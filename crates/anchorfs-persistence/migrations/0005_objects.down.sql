DROP TRIGGER manifest_chunks_immutable ON manifest_chunks; DROP TRIGGER manifests_immutable ON manifests; DROP FUNCTION reject_immutable_update();
ALTER TABLE inodes DROP CONSTRAINT inodes_current_manifest_fk; DROP TABLE manifest_chunks; DROP TABLE manifests; DROP TABLE objects; DROP TABLE upload_reservations;
