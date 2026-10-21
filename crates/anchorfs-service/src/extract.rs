use crate::{
    error::{ApiError, map_rejection},
    middleware::TraceId,
};
use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::request::Parts,
};
use serde::de::DeserializeOwned;
fn trace(parts: &Parts) -> String {
    parts
        .extensions
        .get::<TraceId>()
        .expect("trace middleware")
        .0
        .clone()
}
pub struct ApiPath<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let id = trace(parts);
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct ApiQuery<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let id = trace(parts);
        // serde_urlencoded otherwise replaces invalid UTF-8. Check the decoded bytes once
        // before typed decoding so malformed paths cannot become replacement-character names.
        if !query_is_utf8(parts.uri.query().unwrap_or("")) {
            return Err(map_rejection(axum::http::StatusCode::BAD_REQUEST, id));
        }

        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct ApiJson<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let id = req
            .extensions()
            .get::<TraceId>()
            .expect("trace middleware")
            .0
            .clone();
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(v)| Self(v))
            .map_err(|r| map_rejection(r.status(), id))
    }
}
pub struct IdempotencyKey(pub String);
impl<S: Send + Sync> FromRequestParts<S> for IdempotencyKey {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("Idempotency-Key")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 256 && v.bytes().all(|b| b.is_ascii_graphic()));
        value
            .map(|v| Self(v.to_owned()))
            .ok_or_else(|| map_rejection(axum::http::StatusCode::BAD_REQUEST, trace(parts)))
    }
}
pub struct BinaryBody(pub bytes::Bytes);
pub struct BinaryStream {
    pub length: u64,
    pub sha256: anchorfs_core::Sha256Digest,
    pub body: axum::body::Body,
}
impl<S: Send + Sync> FromRequest<S> for BinaryStream {
    type Rejection = ApiError;
    async fn from_request(req: Request, _: &S) -> Result<Self, ApiError> {
        let trace = req
            .extensions()
            .get::<TraceId>()
            .expect("trace middleware")
            .0
            .clone();
        if req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != Some("application/octet-stream")
        {
            return Err(map_rejection(
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                trace,
            ));
        }
        let length = req
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| {
                let mut error = ApiError::new(anchorfs_core::ErrorCode::InvalidInput, &trace);
                error.message = "content_length_required".into();
                error
            })?;
        let sha256 = req
            .headers()
            .get("X-Anchorfs-Content-SHA256")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| ApiError::new(anchorfs_core::ErrorCode::InvalidInput, &trace))?;
        Ok(Self {
            length,
            sha256,
            body: req.into_body(),
        })
    }
}
impl<S: Send + Sync> FromRequest<S> for BinaryBody {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let id = req
            .extensions()
            .get::<TraceId>()
            .expect("trace middleware")
            .0
            .clone();
        if req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != Some("application/octet-stream")
        {
            return Err(map_rejection(
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                id,
            ));
        }
        axum::body::Bytes::from_request(req, state)
            .await
            .map(Self)
            .map_err(|r| map_rejection(r.status(), id))
    }
}

/// Shape validation only. Stage 1 checks the authenticated actor, workspace, epochs and expiry.
pub struct SessionBinding {
    pub session_id: uuid::Uuid,
    pub session_epoch: i64,
}
fn header<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts.headers.get(name)?.to_str().ok()
}
fn positive_epoch(value: Option<&str>) -> Option<i64> {
    value?.parse::<i64>().ok().filter(|epoch| *epoch > 0)
}
fn workflow_proof(parts: &Parts, required: bool) -> Result<(), ApiError> {
    let token = header(parts, "X-Anchorfs-Workflow-Token");
    let epoch = header(parts, "X-Anchorfs-Workflow-Epoch");
    let absent = !parts.headers.contains_key("X-Anchorfs-Workflow-Token")
        && !parts.headers.contains_key("X-Anchorfs-Workflow-Epoch");
    if (!required && absent)
        || (token.is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            && positive_epoch(epoch).is_some())
    {
        Ok(())
    } else {
        Err(map_rejection(
            axum::http::StatusCode::BAD_REQUEST,
            trace(parts),
        ))
    }
}
impl<S: Send + Sync> FromRequestParts<S> for SessionBinding {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let session_id = header(parts, "X-Anchorfs-Session-Id").and_then(|v| v.parse().ok());
        let session_epoch = positive_epoch(header(parts, "X-Anchorfs-Session-Epoch"));
        workflow_proof(parts, false)?;
        match (session_id, session_epoch) {
            (Some(session_id), Some(session_epoch)) => Ok(Self {
                session_id,
                session_epoch,
            }),
            _ => Err(map_rejection(
                axum::http::StatusCode::BAD_REQUEST,
                trace(parts),
            )),
        }
    }
}
pub struct RequiredWorkflowGrant(pub anchorfs_persistence::authority::WorkflowProof);
impl<S: Send + Sync> FromRequestParts<S> for RequiredWorkflowGrant {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        workflow_proof(parts, true)?;
        let OptionalWorkflowGrant(proof) =
            OptionalWorkflowGrant::from_request_parts(parts, &()).await?;
        Ok(Self(proof.expect("required proof validated")))
    }
}

fn query_is_utf8(query: &str) -> bool {
    let raw = query.as_bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            if i + 2 >= raw.len() {
                return false;
            }
            let hex = |b: u8| char::from(b).to_digit(16);
            let (Some(high), Some(low)) = (hex(raw[i + 1]), hex(raw[i + 2])) else {
                return false;
            };
            decoded.push((high * 16 + low) as u8);
            i += 3;
        } else {
            decoded.push(raw[i]);
            i += 1;
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}

/// Parse the bearer grammar only; transactions verify the proof under a row lock.
pub struct Bearer(pub anchorfs_core::CredentialProof);
impl<S: Send + Sync> FromRequestParts<S> for Bearer {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let presented = parts
            .headers
            .contains_key(axum::http::header::AUTHORIZATION);
        let proof = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .and_then(|(_, value)| value.parse().ok());
        proof.map(Self).ok_or_else(|| {
            let mut error = ApiError::new(anchorfs_core::ErrorCode::Unauthenticated, trace(parts));
            error.token_presented = presented;
            error
        })
    }
}

/// Request timing and trace travel together through control-plane handlers.
pub struct RequestContext {
    pub trace: TraceId,
    pub start: crate::middleware::RequestStart,
}
impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            trace: parts
                .extensions
                .get::<TraceId>()
                .expect("trace middleware")
                .clone(),
            start: *parts
                .extensions
                .get::<crate::middleware::RequestStart>()
                .expect("timing middleware"),
        })
    }
}

pub struct OptionalWorkflowGrant(pub Option<anchorfs_persistence::authority::WorkflowProof>);
impl<S: Send + Sync> FromRequestParts<S> for OptionalWorkflowGrant {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        workflow_proof(parts, false)?;
        let proof = header(parts, "X-Anchorfs-Workflow-Token").map(|token| {
            anchorfs_persistence::authority::WorkflowProof {
                token: anchorfs_core::decode_hex_32(token).expect("validated workflow token"),
                epoch: positive_epoch(header(parts, "X-Anchorfs-Workflow-Epoch"))
                    .expect("validated epoch"),
            }
        });
        Ok(Self(proof))
    }
}

/// Common namespace authority header grammar; the prologue verifies database state.
pub struct SessionAuthority {
    pub binding: SessionBinding,
    pub workflow: Option<anchorfs_persistence::authority::WorkflowProof>,
    pub proof: anchorfs_core::CredentialProof,
}

/// Binary contract validation can finish before reporting a missing bearer.
/// Session/workflow header grammar is still rejected immediately.
pub struct ByteAuthority {
    binding: SessionBinding,
    workflow: Option<anchorfs_persistence::authority::WorkflowProof>,
    proof: Result<anchorfs_core::CredentialProof, ApiError>,
}
impl ByteAuthority {
    pub fn finish(self) -> Result<SessionAuthority, ApiError> {
        Ok(SessionAuthority {
            binding: self.binding,
            workflow: self.workflow,
            proof: self.proof?,
        })
    }
}
impl<S: Send + Sync> FromRequestParts<S> for ByteAuthority {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let binding = SessionBinding::from_request_parts(parts, state).await?;
        let OptionalWorkflowGrant(workflow) =
            OptionalWorkflowGrant::from_request_parts(parts, state).await?;
        let proof = Bearer::from_request_parts(parts, state)
            .await
            .map(|Bearer(proof)| proof);
        Ok(Self {
            binding,
            workflow,
            proof,
        })
    }
}
impl<S: Send + Sync> FromRequestParts<S> for SessionAuthority {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let binding = SessionBinding::from_request_parts(parts, state).await?;
        let OptionalWorkflowGrant(workflow) =
            OptionalWorkflowGrant::from_request_parts(parts, state).await?;
        let Bearer(proof) = Bearer::from_request_parts(parts, state).await?;
        Ok(Self {
            binding,
            workflow,
            proof,
        })
    }
}

#[cfg(test)]
mod bearer_tests {
    use super::*;
    #[tokio::test]
    async fn bearer_grammar_and_challenges() {
        use axum::{
            http::{Request, StatusCode},
            response::IntoResponse,
        };
        let valid = format!("bearer {}.{}", uuid::Uuid::new_v4(), "a".repeat(64));
        for (header, accepted, challenge) in [
            (None, false, "Bearer"),
            (Some("Basic abc"), false, "Bearer error=\"invalid_token\""),
            (
                Some("Bearer missing-dot"),
                false,
                "Bearer error=\"invalid_token\"",
            ),
            (Some(valid.as_str()), true, ""),
        ] {
            let mut request = Request::builder();
            if let Some(header) = header {
                request = request.header("Authorization", header);
            }
            let (mut parts, _) = request.body(()).unwrap().into_parts();
            parts.extensions.insert(TraceId("synthetic".into()));
            let result = Bearer::from_request_parts(&mut parts, &()).await;
            assert_eq!(result.is_ok(), accepted);
            if let Err(error) = result {
                let response = error.into_response();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(response.headers()["WWW-Authenticate"], challenge);
            }
        }
    }
}

#[cfg(test)]
mod lease_extractor_tests {
    use super::*;
    #[tokio::test]
    async fn required_workflow_grant_decodes_and_validates() {
        for (token, epoch, valid) in [
            (Some("ab".repeat(32)), Some("2"), true),
            (None, Some("2"), false),
            (Some("ab".repeat(32)), None, false),
            (Some("bad".into()), Some("2"), false),
            (Some("ab".repeat(32)), Some("0"), false),
        ] {
            let mut req = axum::http::Request::builder();
            if let Some(t) = token {
                req = req.header("X-Anchorfs-Workflow-Token", t);
            }
            if let Some(e) = epoch {
                req = req.header("X-Anchorfs-Workflow-Epoch", e);
            }
            let (mut parts, _) = req.body(()).unwrap().into_parts();
            let result = RequiredWorkflowGrant::from_request_parts(&mut parts, &()).await;
            if valid {
                let proof = result.unwrap().0;
                assert_eq!(proof.token, [171; 32]);
                assert_eq!(proof.epoch, 2);
                assert!(
                    OptionalWorkflowGrant::from_request_parts(&mut parts, &())
                        .await
                        .unwrap()
                        .0
                        .is_some()
                );
            } else {
                assert_eq!(
                    result.err().unwrap().code,
                    anchorfs_core::ErrorCode::InvalidInput
                );
            }
        }
    }
}
