use anchorfs_service::api_doc::document;
use std::{collections::BTreeSet, path::PathBuf};
fn committed() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/openapi-v1.yaml"),
    )
    .unwrap()
}
#[tokio::test]
async fn committed_document_matches_generated() {
    assert_eq!(
        committed().trim_end(),
        document().to_yaml().unwrap().trim_end(),
        "re-run anchorfs-service openapi --format yaml"
    );
}
#[tokio::test]
async fn required_operations_present() {
    let doc: serde_json::Value = serde_json::from_str(&document().to_json().unwrap()).unwrap();
    let actual: BTreeSet<_> = doc["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|path| path.as_object().unwrap().values())
        .map(|operation| operation["operationId"].as_str().unwrap().to_owned())
        .collect();
    let required = [
        "createWorkspace",
        "getWorkspace",
        "createCredential",
        "rotateCredential",
        "revokeCredential",
        "createSession",
        "renewSession",
        "rebindSession",
        "revokeSession",
        "statPath",
        "listDirectory",
        "createDirectory",
        "removeDirectory",
        "openHandle",
        "renewHandle",
        "closeHandle",
        "readRange",
        "writeAt",
        "append",
        "truncate",
        "fsync",
        "createReadPin",
        "releaseReadPin",
        "replaceFile",
        "unlink",
        "createLink",
        "createSymlink",
        "readSymlink",
        "rename",
        "setAttributes",
        "getOperation",
        "findOperation",
        "getAdminOperation",
        "findAdminOperation",
        "acquireWorkflowLease",
        "renewWorkflowLease",
        "releaseWorkflowLease",
        "takeoverWorkflowLease",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(actual, required);
    let errors: BTreeSet<_> = doc["components"]["schemas"]["ErrorCodeDto"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        errors,
        anchorfs_core::ErrorCode::ALL
            .iter()
            .map(|c| c.as_str())
            .collect()
    );
}
#[tokio::test]
async fn document_is_valid_openapi_3_1() {
    let doc: serde_json::Value = serde_json::from_str(&document().to_json().unwrap()).unwrap();
    assert!(doc["openapi"].as_str().unwrap().starts_with("3.1"));
    assert_eq!(doc["info"]["version"], anchorfs_core::API_DOCUMENT_VERSION);
    assert_eq!(
        doc["components"]["securitySchemes"]["credential"]["scheme"],
        "bearer"
    );
    for path in doc["paths"].as_object().unwrap().values() {
        for op in path.as_object().unwrap().values() {
            assert_eq!(op["security"][0]["credential"], serde_json::json!([]));
        }
    }
}

#[test]
fn future_success_contracts_and_authority_are_explicit() {
    let doc: serde_json::Value = serde_json::from_str(&document().to_json().unwrap()).unwrap();
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for op in methods.as_object().unwrap().values() {
            let id = op["operationId"].as_str().unwrap();
            let successes: Vec<_> = op["responses"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(status, _)| status.starts_with('2'))
                .collect();
            assert_eq!(successes.len(), 1, "{id}");
            let success = successes[0].1;
            if id == "readRange" {
                assert!(success["content"].get("application/octet-stream").is_some());
                assert!(success["headers"].get("X-Anchorfs-Generation").is_some());
            } else {
                let reference = success["content"]["application/json"]["schema"]["$ref"]
                    .as_str()
                    .unwrap();
                assert!(
                    doc.pointer(reference.strip_prefix('#').unwrap()).is_some(),
                    "{id}: {reference}"
                );
            }
            let needs_session = path.contains("{workspace_id}")
                && ![
                    "getWorkspace",
                    "createSession",
                    "getOperation",
                    "findOperation",
                ]
                .contains(&id);
            let params = op["parameters"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for header in ["X-Anchorfs-Session-Id", "X-Anchorfs-Session-Epoch"] {
                assert_eq!(
                    params
                        .iter()
                        .any(|p| p["name"] == header && p["required"] == true),
                    needs_session,
                    "{id}: {header}"
                );
            }
        }
    }
    let stat = &doc["paths"]["/v1/workspaces/{workspace_id}/stat"]["get"];
    assert!(
        stat["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "path" && p["required"] == false)
    );
}
