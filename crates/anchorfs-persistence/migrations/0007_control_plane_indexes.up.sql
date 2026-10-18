CREATE INDEX credentials_issued_by_idx ON credentials(issued_by_credential_id) WHERE issued_by_credential_id IS NOT NULL;
CREATE INDEX client_sessions_credential_active_idx ON client_sessions(credential_id) WHERE revoked_at IS NULL;
