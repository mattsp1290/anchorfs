#![allow(dead_code)]
use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, Response, StatusCode, header},
    response::IntoResponse,
};
use reqwest::Url;
use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot},
    task::JoinHandle,
};

#[derive(Debug, Clone)]
pub enum Fault {
    Pass,
    Status {
        status: u16,
        code: &'static str,
    },
    ForwardThenStatus {
        status: u16,
    },
    Delay(Duration),
    DelayForward(Duration),
    SwallowResponse,
    DropRequest,
    TruncateBody {
        keep: usize,
    },
    AbortMidBody {
        after: usize,
    },
    FlipRequestBodyByte {
        index: usize,
    },
    StripResponseHeader {
        name: &'static str,
    },
    MutateResponseHeader {
        name: &'static str,
        value: &'static str,
    },
}
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: Method,
    pub path: String,
    pub query: String,
    pub request_headers: HeaderMap,
    pub body_len: usize,
    pub fault: Fault,
    pub forwarded: bool,
    pub upstream_status: Option<u16>,
}
pub type Matcher = fn(&Method, &str, &str) -> bool;
type Scripts = Arc<Mutex<Vec<(Matcher, VecDeque<Fault>)>>>;
#[derive(Clone)]
struct ProxyState {
    upstream: Url,
    client: reqwest::Client,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    scripts: Scripts,
}
pub struct Proxy {
    pub endpoint: Url,
    pub recorded: Arc<Mutex<Vec<Recorded>>>,
    scripts: Scripts,
    server: JoinHandle<()>,
}
impl Proxy {
    pub async fn start(upstream: &Url) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let scripts = Arc::new(Mutex::new(Vec::new()));
        let state = ProxyState {
            upstream: upstream.clone(),
            client: reqwest::Client::builder()
                .http1_only()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            recorded: recorded.clone(),
            scripts: scripts.clone(),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().fallback(handler).with_state(state))
                .await
                .unwrap();
        });
        Self {
            endpoint,
            recorded,
            scripts,
            server,
        }
    }
    pub fn script_for(&self, matcher: Matcher, faults: impl IntoIterator<Item = Fault>) {
        self.scripts
            .lock()
            .unwrap()
            .push((matcher, faults.into_iter().collect()));
    }
    pub fn count(&self, matcher: Matcher) -> usize {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matcher(&r.method, &r.path, &r.query))
            .count()
    }
    pub fn forwarded(&self, matcher: Matcher) -> usize {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.forwarded && matcher(&r.method, &r.path, &r.query))
            .count()
    }
    pub async fn wait_forwarded(&self, matcher: Matcher, n: usize, deadline: Duration) -> bool {
        let until = Instant::now() + deadline;
        loop {
            if self.forwarded(matcher) >= n {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub fn is_put(method: &Method, _path: &str, _query: &str) -> bool {
    *method == Method::PUT
}
pub fn is_head_object(method: &Method, path: &str, _query: &str) -> bool {
    *method == Method::HEAD && path.trim_matches('/').contains('/')
}
pub fn is_get_object(method: &Method, path: &str, query: &str) -> bool {
    *method == Method::GET && path.trim_matches('/').contains('/') && !is_list(method, path, query)
}
pub fn is_list(method: &Method, path: &str, query: &str) -> bool {
    *method == Method::GET
        && !path.trim_matches('/').contains('/')
        && reqwest::Url::parse(&format!("http://fixture.invalid/?{query}"))
            .unwrap()
            .query_pairs()
            .any(|(key, value)| key == "list-type" && value == "2")
}
fn status_response(status: u16, code: &str) -> Response<Body> {
    (
        StatusCode::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/xml")],
        format!("<Error><Code>{code}</Code></Error>"),
    )
        .into_response()
}
async fn hang() -> Response<Body> {
    Notify::new().notified().await;
    unreachable!("unsignalled fault")
}
async fn handler(State(state): State<ProxyState>, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let query = parts.uri.query().unwrap_or("").to_owned();
    let mut bytes = to_bytes(body, 4 * 1024 * 1024 + 64 * 1024)
        .await
        .unwrap()
        .to_vec();
    let fault = {
        let mut scripts = state.scripts.lock().unwrap();
        scripts
            .iter_mut()
            .find(|(matcher, _)| matcher(&parts.method, &path, &query))
            .and_then(|(_, faults)| faults.pop_front())
            .unwrap_or(Fault::Pass)
    };
    let index = {
        let mut records = state.recorded.lock().unwrap();
        let index = records.len();
        records.push(Recorded {
            method: parts.method.clone(),
            path: path.clone(),
            query: query.clone(),
            request_headers: parts.headers.clone(),
            body_len: bytes.len(),
            fault: fault.clone(),
            forwarded: false,
            upstream_status: None,
        });
        index
    };
    match &fault {
        Fault::Status { status, code } => return status_response(*status, code),
        Fault::DropRequest => return hang().await,
        Fault::FlipRequestBodyByte { index } => {
            *bytes
                .get_mut(*index)
                .expect("fault index within synthetic body") ^= 0xff;
        }
        _ => {}
    }
    let (send, receive) = oneshot::channel();
    // The exchange lives independently of the request handler, including DelayForward.
    tokio::spawn(async move {
        if let Fault::DelayForward(delay) = fault {
            tokio::time::sleep(delay).await;
        }
        let path_query = if query.is_empty() {
            path
        } else {
            format!("{path}?{query}")
        };
        let url = format!(
            "{}{}",
            state.upstream.as_str().trim_end_matches('/'),
            path_query
        );
        let result = async {
            // Preserve the signed Host and all other request headers explicitly.
            let upstream = state
                .client
                .request(parts.method, url)
                .headers(parts.headers)
                .body(bytes)
                .send()
                .await?;
            let status = upstream.status();
            let headers = upstream.headers().clone();
            let bytes = upstream.bytes().await?;
            Ok::<_, reqwest::Error>((status, headers, bytes))
        }
        .await;
        let response = match result {
            Err(error) => status_response(
                502,
                if error.is_timeout() {
                    "Timeout"
                } else {
                    "ProxyUpstreamError"
                },
            ),
            Ok((status, mut headers, bytes)) => {
                {
                    let mut recorded = state.recorded.lock().unwrap();
                    recorded[index].forwarded = true;
                    recorded[index].upstream_status = Some(status.as_u16());
                }
                if let Fault::Delay(delay) = fault {
                    tokio::time::sleep(delay).await;
                }
                match fault {
                    Fault::SwallowResponse => {
                        drop(send);
                        return;
                    }
                    Fault::ForwardThenStatus { status } => status_response(status, "BadGateway"),
                    _ => {
                        let body = match fault {
                            Fault::TruncateBody { keep } => {
                                let keep = keep.min(bytes.len());
                                headers.remove(header::TRANSFER_ENCODING);
                                headers.insert(
                                    header::CONTENT_LENGTH,
                                    keep.to_string().parse().unwrap(),
                                );
                                Body::from(bytes.slice(..keep))
                            }
                            Fault::AbortMidBody { after } => {
                                let after = after.min(bytes.len());
                                Body::from_stream(AbortStream {
                                    bytes: Some(bytes.slice(..after)),
                                    failed: false,
                                    delay: Box::pin(tokio::time::sleep(Duration::from_millis(50))),
                                })
                            }
                            Fault::StripResponseHeader { name } => {
                                headers.remove(name);
                                Body::from(bytes)
                            }
                            Fault::MutateResponseHeader { name, value } => {
                                headers.insert(
                                    header::HeaderName::from_static(name),
                                    value.parse().unwrap(),
                                );
                                Body::from(bytes)
                            }
                            _ => Body::from(bytes),
                        };
                        let mut response = Response::new(body);
                        *response.status_mut() = status;
                        *response.headers_mut() = headers;
                        response
                    }
                }
            }
        };
        let _ = send.send(response);
    });
    match receive.await {
        Ok(response) => response,
        // SwallowResponse deliberately completes forwarding while withholding the reply.
        Err(_) => hang().await,
    }
}
struct AbortStream {
    bytes: Option<Bytes>,
    failed: bool,
    delay: Pin<Box<tokio::time::Sleep>>,
}
impl futures_core::Stream for AbortStream {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(bytes) = self.bytes.take() {
            return Poll::Ready(Some(Ok(bytes)));
        }
        // Give HTTP/1.1 a chance to flush the headers and the first body frame.
        // An immediate error may suppress the response entirely and trigger an SDK retry.
        if self.delay.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        if !self.failed {
            self.failed = true;
            Poll::Ready(Some(Err(io::Error::other(
                "synthetic interrupted object body",
            ))))
        } else {
            Poll::Ready(None)
        }
    }
}
