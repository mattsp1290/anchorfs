use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}
impl Reply {
    pub fn status(self, expected: u16) -> Self {
        assert_eq!(self.status.as_u16(), expected, "{}", self.body);
        self
    }
    pub fn error(self, expected: u16, code: &str) -> Self {
        let r = self.status(expected);
        assert_eq!(r.body["code"], code);
        r
    }
}
#[derive(Clone)]
pub struct Credential {
    pub id: Uuid,
    pub secret: String,
}
impl Credential {
    pub fn from_grant(value: &Value) -> Self {
        Self {
            id: Uuid::parse_str(value["credential_id"].as_str().unwrap()).unwrap(),
            secret: value["secret"].as_str().unwrap().into(),
        }
    }
}
pub struct Client {
    pub app: Router,
}
impl Client {
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        credential: Option<&Credential>,
        session: Option<&Value>,
        key: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(c) = credential {
            request = request.header("Authorization", format!("Bearer {}.{}", c.id, c.secret));
        }
        if let Some(session) = session {
            request = request
                .header(
                    "X-Anchorfs-Session-Id",
                    session["session_id"].as_str().unwrap(),
                )
                .header(
                    "X-Anchorfs-Session-Epoch",
                    session["session_epoch"].as_i64().unwrap(),
                );
        }
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&body).unwrap())
            }
            None => Body::empty(),
        };
        self.raw(request.body(body).unwrap()).await
    }
    pub async fn raw(&self, request: Request<Body>) -> Reply {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap())
                .unwrap();
        if body.get("code").is_some() {
            assert_eq!(
                body["trace_id"],
                headers["X-Anchorfs-Trace-Id"].to_str().unwrap()
            );
        }
        Reply {
            status,
            headers,
            body,
        }
    }
}
