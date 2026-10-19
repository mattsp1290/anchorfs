ALTER TABLE inodes ADD COLUMN charge_released_at timestamptz;
CREATE INDEX handles_open_session_idx ON handles(workspace_id,client_session_id) WHERE closed_at IS NULL;
CREATE INDEX handles_open_inode_idx ON handles(workspace_id,inode_id) WHERE closed_at IS NULL;
