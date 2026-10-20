//! Persisted-state accounting oracle, independent of wall-clock handle expiry.
use anchorfs_core::WorkspaceId;
use sqlx::{Executor, Postgres};
/// Returns explicit violations; the root is deliberately uncharged.
pub async fn check<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    workspace: WorkspaceId,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(r#"
WITH nodes AS (
 SELECT i.*, w.root_inode_id,
 (SELECT count(*) FROM directory_entries e WHERE e.workspace_id=i.workspace_id AND e.inode_id=i.id) AS entries,
 (SELECT count(*) FROM directory_entries e JOIN inodes c ON c.workspace_id=e.workspace_id AND c.id=e.inode_id WHERE e.workspace_id=i.workspace_id AND e.parent_inode_id=i.id AND c.kind='directory') AS child_dirs
 FROM inodes i JOIN workspaces w ON w.id=i.workspace_id WHERE i.workspace_id=$1
), violations AS (
 SELECT 'nlink:' || id AS violation FROM nodes WHERE nlink <> CASE WHEN kind='directory' THEN CASE WHEN unlinked_at IS NOT NULL THEN 0 ELSE 2+child_dirs END ELSE entries END
 UNION ALL SELECT 'tombstone:' || id FROM nodes WHERE id<>root_inode_id AND ((nlink=0)<>(unlinked_at IS NOT NULL))
 UNION ALL SELECT 'released-linked:' || id FROM nodes WHERE charge_released_at IS NOT NULL AND nlink<>0
 UNION ALL SELECT 'entry-tombstone:' || id FROM nodes WHERE nlink=0 AND entries<>0
 UNION ALL SELECT 'directory-parent:' || id FROM nodes WHERE kind='directory' AND id<>root_inode_id AND nlink>0 AND entries<>1
 UNION ALL SELECT 'invalid-parent:' || e.parent_inode_id FROM directory_entries e JOIN inodes p ON p.workspace_id=e.workspace_id AND p.id=e.parent_inode_id WHERE e.workspace_id=$1 AND (p.kind<>'directory' OR p.nlink=0)
 UNION ALL SELECT 'logical-inodes' FROM workspace_quotas q WHERE q.workspace_id=$1 AND q.logical_inodes_used<>(SELECT count(*) FROM nodes WHERE id<>root_inode_id AND charge_released_at IS NULL)
 UNION ALL SELECT 'logical-bytes' FROM workspace_quotas q WHERE q.workspace_id=$1 AND q.logical_bytes_used<>(SELECT coalesce(sum(size),0) FROM nodes WHERE id<>root_inode_id AND charge_released_at IS NULL)
 UNION ALL SELECT 'file-manifest:' || id FROM nodes WHERE kind='file' AND current_manifest_id IS NULL
 UNION ALL SELECT 'manifest-size:' || n.id FROM nodes n JOIN manifests m ON (m.workspace_id,m.id)=(n.workspace_id,n.current_manifest_id) WHERE m.size<>n.size
 UNION ALL SELECT 'manifest-generation:' || n.id FROM nodes n JOIN manifests m ON (m.workspace_id,m.id)=(n.workspace_id,n.current_manifest_id) WHERE m.generation>n.generation
 UNION ALL SELECT DISTINCT 'chunk-extent:' || c.manifest_id FROM manifest_chunks c JOIN manifests m ON (m.workspace_id,m.id)=(c.workspace_id,c.manifest_id) JOIN objects o ON (o.workspace_id,o.id)=(c.workspace_id,c.object_id) WHERE c.workspace_id=$1 AND (c.chunk_index::bigint*c.chunk_size+c.logical_length>m.size OR c.logical_length>o.byte_length)
 UNION ALL SELECT DISTINCT 'chunk-object:' || c.manifest_id FROM manifest_chunks c JOIN objects o ON (o.workspace_id,o.id)=(c.workspace_id,c.object_id) WHERE c.workspace_id=$1 AND o.state<>'published'
 UNION ALL SELECT 'candidate-published:' || c.object_id FROM deletion_candidates c JOIN objects o ON (o.workspace_id,o.id)=(c.workspace_id,c.object_id) WHERE c.workspace_id=$1 AND o.state='published'
 UNION ALL SELECT 'staging' FROM workspace_quotas q WHERE q.workspace_id=$1 AND q.staging_bytes_reserved<>(SELECT coalesce(sum(reserved_bytes),0) FROM upload_reservations WHERE workspace_id=$1 AND state='active')
 UNION ALL SELECT 'physical' FROM workspace_quotas q WHERE q.workspace_id=$1 AND q.physical_bytes_used<>((SELECT coalesce(sum(byte_length),0) FROM objects WHERE workspace_id=$1 AND state='published')+(SELECT coalesce(sum(physical_bytes),0) FROM deletion_candidates WHERE workspace_id=$1))
) SELECT violation FROM violations ORDER BY violation
"#).bind(workspace.0).fetch_all(executor).await
}

pub async fn check_deployment<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT 'deployment-staging' FROM deployment_quotas d WHERE singleton AND staging_bytes_reserved<>(SELECT coalesce(sum(staging_bytes_reserved),0) FROM workspace_quotas) UNION ALL SELECT 'deployment-physical' FROM deployment_quotas d WHERE singleton AND physical_bytes_used<>(SELECT coalesce(sum(physical_bytes_used),0) FROM workspace_quotas) ORDER BY 1")
        .fetch_all(executor).await
}
