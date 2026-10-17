ALTER TABLE manifests DROP CONSTRAINT manifests_operation_fk; ALTER TABLE objects DROP CONSTRAINT objects_reservation_fk; ALTER TABLE upload_reservations DROP CONSTRAINT upload_reservations_operation_fk;
DROP TABLE deletion_candidates; DROP TABLE read_pins; DROP TABLE handles; DROP TABLE operations;
