ALTER TABLE admin_operations DROP CONSTRAINT admin_operations_credential_fk;
ALTER TABLE workspaces DROP CONSTRAINT workspaces_workflow_owner_fk;
DROP TABLE client_sessions;
DROP TRIGGER credentials_issuer_scope ON credentials;
DROP FUNCTION check_credentials_issuer_scope();
DROP TABLE credentials;
