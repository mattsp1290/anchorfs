use super::*;
use anchorfs_core::{Secret, SessionId};
use auth::*;

#[test]
fn client_headers_and_plaintext_policy() {
    assert!(Endpoint::parse("http://localhost:1234", false).is_err());
    assert!(Endpoint::parse("https://localhost:1234", false).is_ok());
    assert!(Endpoint::parse("http://localhost:1234", true).is_ok());
    for invalid in [
        "ftp://localhost",
        "https://user:password@localhost",
        "https://localhost/api",
        "https://localhost/?secret=bad",
    ] {
        assert!(Endpoint::parse(invalid, true).is_err());
    }
    assert!(Bearer::parse("invalid").is_err());
    let credential_id = uuid::Uuid::new_v4();
    let token = "ab".repeat(32);
    let bearer_text = format!("{credential_id}.{token}");
    let bearer = Bearer::parse(&bearer_text).unwrap();
    assert_eq!(bearer.credential_id.0, credential_id);
    assert!(!format!("{bearer:?}").contains(&token));
    let mut authority = Authority {
        bearer,
        session: None,
        workflow: None,
    };
    for mode in 0..3 {
        if mode >= 1 {
            authority.session = Some(SessionAuthority {
                session_id: SessionId::new(),
                session_epoch: 7,
            });
        }
        if mode >= 2 {
            authority.workflow = Some(WorkflowAuthority {
                token: Secret::new(token.clone()),
                epoch: 9,
            });
        }
        let request = authority
            .apply(reqwest::Client::new().get("https://localhost"))
            .build()
            .unwrap();
        let headers = request.headers();
        assert_eq!(headers["Authorization"], format!("Bearer {bearer_text}"));
        assert!(headers["Authorization"].is_sensitive());
        assert_eq!(headers.contains_key("X-Anchorfs-Session-Id"), mode >= 1);
        assert_eq!(headers.contains_key("X-Anchorfs-Workflow-Token"), mode >= 2);
        assert_eq!(headers.contains_key("X-Anchorfs-Workflow-Epoch"), mode >= 2);
        if mode >= 1 {
            assert_eq!(headers["X-Anchorfs-Session-Epoch"], "7");
        }
        if mode >= 2 {
            assert_eq!(headers["X-Anchorfs-Workflow-Epoch"], "9");
            assert!(headers["X-Anchorfs-Workflow-Token"].is_sensitive());
        }
    }
}

#[test]
fn errno_hint_matches_core() {
    for code in anchorfs_core::ErrorCode::ALL {
        let error = error::ApiFailure {
            status: code.status(),
            code: code.as_str().into(),
            message: String::new(),
            trace_id: String::new(),
            operation_id: None,
            retry_after_ms: None,
        };
        assert_eq!(error.code.kind(), Some(*code));
        assert_eq!(error.errno_hint(), Some(code.errno_hint()));
    }
}

#[test]
fn unknown_error_codes_remain_forward_tolerant() {
    let error: error::ApiFailure = serde_json::from_value(serde_json::json!({
        "code":"future_code", "message":"future error", "trace_id":"t", "operation_id":null, "retry_after_ms":null
    })).unwrap();
    assert_eq!(error.code.as_str(), "future_code");
    assert_eq!(error.code.kind(), None);
    assert_eq!(error.errno_hint(), None);
}

#[tokio::test]
async fn invalid_wire_time_is_an_input_error_before_any_request() {
    let authority = Authority {
        bearer: Bearer::parse(&format!("{}.{}", uuid::Uuid::new_v4(), "ab".repeat(32))).unwrap(),
        session: None,
        workflow: None,
    };
    let client = Client::new(
        Endpoint::parse("http://127.0.0.1:1", true).unwrap(),
        uuid::Uuid::new_v4(),
        authority,
        None,
    )
    .unwrap();
    let time = time::Date::from_calendar_date(-1, time::Month::January, 1)
        .unwrap()
        .midnight()
        .assume_utc();
    let result = client
        .set_attributes(
            &wire::SetAttributes {
                path: None,
                update: anchorfs_core::AttributeUpdate {
                    mtime: Some(time),
                    ..Default::default()
                },
            },
            &retry::IdempotencyKey::new(),
        )
        .await;
    assert!(matches!(result, Err(error::ClientError::InvalidInput(_))));
}

#[test]
fn readiness_bodies_deserialize() {
    let ready: wire::ReadyzBody = serde_json::from_value(serde_json::json!({
        "tls": true, "schema_version": 3, "object_format_version": 1, "api_version": "v1",
        "durability_profile": "durable", "database": "ok", "backend": "ok",
        "checksum_mode": "sha256", "durability_warnings": ["fsync=off"]
    }))
    .unwrap();
    assert!(ready.tls);
    assert_eq!(ready.durability_profile, "durable");
    assert_eq!(
        ready.durability_warnings.as_deref(),
        Some(&["fsync=off".to_owned()][..])
    );
    let bare: wire::ReadyzBody = serde_json::from_value(serde_json::json!({
        "tls": false, "schema_version": 3, "object_format_version": 1, "api_version": "v1",
        "durability_profile": "development", "database": "ok", "backend": "ok",
        "checksum_mode": "none"
    }))
    .unwrap();
    assert!(bare.durability_warnings.is_none());
    let error: wire::ApiErrorBody = serde_json::from_value(serde_json::json!({
        "code": "dependency_unavailable", "message": "backend: down", "trace_id": "t-1"
    }))
    .unwrap();
    assert_eq!(error.code, "dependency_unavailable");
}

#[tokio::test]
async fn readiness_rejects_zero_timeout_and_bad_ca() {
    let endpoint = Endpoint::parse("https://127.0.0.1:1", false).unwrap();
    assert!(matches!(
        readiness::readiness(&endpoint, None, std::time::Duration::ZERO).await,
        Err(error::ClientError::InvalidInput(_))
    ));
    assert!(matches!(
        readiness::readiness(
            &endpoint,
            Some(b"not pem"),
            std::time::Duration::from_secs(1)
        )
        .await,
        Err(error::ClientError::InvalidInput(_))
    ));
}
