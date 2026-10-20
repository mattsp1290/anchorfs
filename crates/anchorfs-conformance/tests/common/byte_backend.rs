//! Byte fixture backend ownership and an independent raw-bucket accounting oracle.
use super::{fake_backend::FakeBackend, minio, proxy::Proxy};
use anchorfs_backend::traits::ObjectBackend;
use anchorfs_core::DeploymentId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub enum Backend {
    Fake(Arc<FakeBackend>),
    Minio {
        backend: Arc<dyn ObjectBackend>,
        proxy: Option<Arc<Proxy>>,
        deployment: DeploymentId,
        endpoint: reqwest::Url,
    },
}
impl Backend {
    pub fn service(&self) -> Arc<dyn ObjectBackend> {
        match self {
            Self::Fake(b) => b.clone(),
            Self::Minio { backend, .. } => backend.clone(),
        }
    }
    pub fn untouched(&self) {
        if let Self::Fake(backend) = self {
            backend.untouched();
        }
    }
    pub async fn reconcile(&self, pool: &sqlx::PgPool, workspace: uuid::Uuid) {
        let Self::Minio {
            deployment,
            endpoint,
            ..
        } = self
        else {
            return;
        };
        let rows: Vec<(uuid::Uuid, String, i64, bool)> = sqlx::query_as("SELECT o.id,o.state,o.byte_length,(o.state IN ('published','uncertain') OR EXISTS(SELECT 1 FROM deletion_candidates c WHERE c.workspace_id=o.workspace_id AND c.object_id=o.id) OR (o.state='staged' AND r.state='active')) FROM objects o JOIN upload_reservations r ON (r.workspace_id,r.id)=(o.workspace_id,o.reservation_id) WHERE o.workspace_id=$1")
            .bind(workspace).fetch_all(pool).await.unwrap();
        let expected: BTreeMap<_, _> = rows
            .into_iter()
            .map(|(id, state, length, charged)| {
                (
                    format!("{deployment}/{workspace}/{id}"),
                    (state, length, charged),
                )
            })
            .collect();
        // Raw SDK listing bypasses the service adapter and any scripted proxy.
        let client = minio::raw_client(endpoint).await;
        let mut cursor = None;
        let mut observed = BTreeSet::new();
        loop {
            let page = client
                .list_objects_v2()
                .bucket(minio::BUCKET)
                .prefix(format!("{deployment}/{workspace}/"))
                .set_continuation_token(cursor)
                .send()
                .await
                .unwrap();
            for object in page.contents() {
                let key = object.key().expect("listed object key");
                let row = expected
                    .get(key)
                    .unwrap_or_else(|| panic!("bucket-uncharged:{key}"));
                assert!(row.2, "bucket-uncharged:{key}");
                assert_eq!(object.size(), Some(row.1), "bucket-length:{key}");
                observed.insert(key.to_string());
            }
            if !page.is_truncated().unwrap_or(false) {
                break;
            }
            cursor = Some(
                page.next_continuation_token()
                    .expect("truncated list cursor")
                    .into(),
            );
        }
        for (key, (state, _, _)) in expected {
            if state == "published" {
                assert!(observed.contains(&key), "bucket-missing:{key}");
            }
        }
    }
}
