CREATE SEQUENCE inodes_id_seq;
CREATE TABLE inodes (
 workspace_id uuid NOT NULL REFERENCES workspaces(id), id bigint NOT NULL DEFAULT nextval('inodes_id_seq'), generation bigint NOT NULL DEFAULT 1,
 kind text NOT NULL CHECK (kind IN ('file','directory','symlink')), size bigint NOT NULL DEFAULT 0 CHECK (size>=0), mode integer NOT NULL CHECK (mode BETWEEN 0 AND 511), uid bigint NOT NULL CHECK (uid BETWEEN 0 AND 4294967295), gid bigint NOT NULL CHECK (gid BETWEEN 0 AND 4294967295),
 atime timestamptz NOT NULL, mtime timestamptz NOT NULL, ctime timestamptz NOT NULL, nlink integer NOT NULL DEFAULT 1 CHECK (nlink>=0), current_manifest_id uuid, symlink_target text, unlinked_at timestamptz,
 PRIMARY KEY (workspace_id,id), CHECK ((kind='symlink')=(symlink_target IS NOT NULL)), CHECK (kind='file' OR current_manifest_id IS NULL)
);
CREATE TABLE directory_entries (
 workspace_id uuid NOT NULL, parent_inode_id bigint NOT NULL, name text COLLATE "C" NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255 AND position('/' in name)=0 AND name NOT IN ('.','..')), inode_id bigint NOT NULL,
 PRIMARY KEY (workspace_id,parent_inode_id,name),
 CONSTRAINT directory_entries_parent_fk FOREIGN KEY (workspace_id,parent_inode_id) REFERENCES inodes(workspace_id,id),
 CONSTRAINT directory_entries_inode_fk FOREIGN KEY (workspace_id,inode_id) REFERENCES inodes(workspace_id,id)
);
CREATE INDEX directory_entries_inode_idx ON directory_entries(workspace_id,inode_id);
ALTER TABLE workspaces ADD CONSTRAINT workspaces_root_inode_fk FOREIGN KEY (id,root_inode_id) REFERENCES inodes(workspace_id,id) DEFERRABLE INITIALLY DEFERRED;
