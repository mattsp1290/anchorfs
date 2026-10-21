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
        assert_eq!(error.errno_hint(), Some(code.errno_hint()));
    }
}
