use super::support::*;
use crate::common::{
    minio,
    proxy::{self, Fault},
};

#[tokio::test]
async fn by6_proxy_faults_preserve_old_bytes_and_conservative_charges() {
    for mode in ["drop", "lost-reply", "request-integrity", "reply-integrity"] {
        let f = Fixture::with_bytes(workspace_body(), configure, Arc::new(NoFaults), true).await;
        let handle = open(&f, &f.worker, &f.worker_session, "f", true).await;
        write(&f, &handle, "seed", 0, b"safe").await.status(200);
        let Backend::Minio {
            proxy: Some(proxy),
            endpoint,
            deployment,
            ..
        } = &f.backend
        else {
            panic!("proxy expected")
        };
        match mode {
            "drop" => {
                proxy.script_for(proxy::is_put, [Fault::DropRequest, Fault::DropRequest]);
                proxy.script_for(
                    proxy::is_head_object,
                    [
                        Fault::Status {
                            status: 404,
                            code: "NoSuchKey",
                        },
                        Fault::Status {
                            status: 404,
                            code: "NoSuchKey",
                        },
                    ],
                );
            }
            "lost-reply" => proxy.script_for(proxy::is_put, [Fault::SwallowResponse]),
            "request-integrity" => proxy.script_for(
                proxy::is_put,
                [
                    Fault::FlipRequestBodyByte { index: 1 },
                    Fault::FlipRequestBodyByte { index: 1 },
                ],
            ),
            "reply-integrity" => proxy.script_for(
                proxy::is_put,
                [Fault::MutateResponseHeader {
                    name: "x-amz-checksum-sha256",
                    value: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                }],
            ),
            _ => unreachable!(),
        }
        let reply = write(&f, &handle, "fault", 0, b"next").await;
        let operation = if mode == "lost-reply" {
            reply.status(200).json()
        } else {
            error(reply, 503, "dependency_unavailable")
        };
        let operation_id: Uuid = operation["operation_id"].as_str().unwrap().parse().unwrap();
        let (object, state): (Uuid, String) = sqlx::query_as(
            "SELECT o.id,o.state FROM objects o JOIN upload_reservations r ON (r.workspace_id,r.id)=(o.workspace_id,o.reservation_id) WHERE o.workspace_id=$1 AND r.operation_id=$2",
        )
        .bind(f.workspace)
        .bind(operation_id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        let raw = minio::raw_client(endpoint).await;
        let stored = minio::raw_get(&raw, &format!("{deployment}/{}/{object}", f.workspace)).await;
        if mode == "drop" || mode == "request-integrity" {
            assert!(
                stored.is_none(),
                "provider must reject or never receive this object"
            );
        } else {
            assert_eq!(stored.unwrap(), b"next");
        }
        if mode == "lost-reply" {
            assert_eq!(state, "published");
            assert_eq!(
                read(&f, &handle, 0, 4096, None).await.status(200).bytes,
                b"next".as_slice()
            );
        } else {
            assert_eq!(state, "uncertain");
            assert_eq!(
                read(&f, &handle, 0, 4096, None).await.status(200).bytes,
                b"safe".as_slice()
            );
            let view = f
                .client
                .call(
                    "GET",
                    &format!("{}/operations/{operation_id}", f.workspace_path()),
                    Some(&f.host),
                    None,
                    None,
                    None,
                )
                .await
                .status(200)
                .body;
            assert_eq!(view["state"], "failed");
            let charged:i64=sqlx::query_scalar("SELECT physical_bytes FROM deletion_candidates WHERE workspace_id=$1 AND object_id=$2").bind(f.workspace).bind(object).fetch_one(&f.pool).await.unwrap();
            assert_eq!(charged, 4);
        }
        let counts:(i64,i64)=sqlx::query_as("SELECT physical_bytes_used,staging_bytes_reserved FROM workspace_quotas WHERE workspace_id=$1").bind(f.workspace).fetch_one(&f.pool).await.unwrap();
        assert_eq!(counts, (8, 0));
        f.invariants().await;
    }
}
