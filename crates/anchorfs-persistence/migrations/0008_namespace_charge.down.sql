DROP INDEX handles_open_inode_idx;
DROP INDEX handles_open_session_idx;
ALTER TABLE inodes DROP COLUMN charge_released_at;
