//! Synthetic HTTP/1 relay: forwards a request, then optionally discards its full response.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub struct ApiProxy {
    pub endpoint: String,
    pub dropped: Arc<AtomicUsize>,
    pub requests: Arc<Mutex<Vec<String>>>,
    drop_next: Arc<AtomicUsize>,
    delay_next: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl ApiProxy {
    pub async fn start(upstream: std::net::SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let drop_next = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let delay_next = Arc::new(AtomicUsize::new(0));
        let delays = delay_next.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (drops, count, log) = (drop_next.clone(), dropped.clone(), requests.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut downstream, _)) = listener.accept().await else {
                    break;
                };
                let (drops, count, log) = (drops.clone(), count.clone(), log.clone());
                let delays = delays.clone();
                tokio::spawn(async move {
                    let result: std::io::Result<()> = async {
                        let mut incoming = Vec::new();
                        let header_end = loop {
                            let mut buffer = [0u8; 8192];
                            let n = downstream.read(&mut buffer).await?;
                            if n == 0 {
                                return Ok(());
                            }
                            incoming.extend_from_slice(&buffer[..n]);
                            if let Some(at) = incoming.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            assert!(incoming.len() < 65536, "bounded test headers");
                        };
                        let headers = String::from_utf8(incoming[..header_end].to_vec()).unwrap();
                        let mut summary = headers.lines().next().unwrap_or("").to_owned();
                        for line in headers.lines().filter(|line| {
                            line.to_ascii_lowercase().starts_with("idempotency-key:")
                        }) {
                            summary.push('\n');
                            summary.push_str(line);
                        }
                        log.lock().unwrap().push(summary);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        while incoming.len() < header_end + length {
                            let mut buffer = [0u8; 8192];
                            let n = downstream.read(&mut buffer).await?;
                            if n == 0 {
                                return Ok(());
                            }
                            incoming.extend_from_slice(&buffer[..n]);
                        }
                        let mut upstream = TcpStream::connect(upstream).await?;
                        let mut forward = headers
                            .lines()
                            .filter(|line| {
                                !line.to_ascii_lowercase().starts_with("connection:")
                                    && !line.is_empty()
                            })
                            .collect::<Vec<_>>()
                            .join("\r\n");
                        forward.push_str("\r\nConnection: close\r\n\r\n");
                        upstream.write_all(forward.as_bytes()).await?;
                        upstream
                            .write_all(&incoming[header_end..header_end + length])
                            .await?;
                        let mut response = Vec::new();
                        upstream.read_to_end(&mut response).await?;
                        if delays
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                            .is_ok()
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(1700)).await;
                        }
                        if drops
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                            .is_ok()
                        {
                            count.fetch_add(1, Ordering::SeqCst);
                        } else {
                            downstream.write_all(&response).await?;
                        }
                        downstream.shutdown().await
                    }
                    .await;
                    let _ = result; // A deliberately dropped client connection is expected.
                });
            }
        });
        Self {
            endpoint,
            dropped,
            requests,
            drop_next,
            delay_next,
            task,
        }
    }
    pub fn drop_responses(&self, n: usize) {
        self.drop_next.store(n, Ordering::SeqCst);
    }
    pub fn delay_responses(&self, n: usize) {
        self.delay_next.store(n, Ordering::SeqCst);
    }
    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for ApiProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
