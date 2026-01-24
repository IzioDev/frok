use bytes::Bytes;
use futures_util::stream::FuturesUnordered;
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Incoming;
use hyper::client::conn::{http1, http2};
use hyper::{Method, Request, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::collections::HashMap;
use std::convert::Infallible;
use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_stream::{Stream, StreamExt};
use tracing::warn;

use frok_common::http::{header_bytes_eq, is_hop_header_bytes};
use frok_protocol::{HttpRequestStart, IngressMode};

pub struct HttpProxy {
    workers: RwLock<HashMap<WorkerKey, Arc<WorkerPool>>>,
    config: ProxyConfig,
}

pub type BodyStream = Pin<Box<dyn Stream<Item = Bytes> + Send + 'static>>;

const DEFAULT_HTTP1_POOL_SIZE: usize = 4;
const DEFAULT_REQUEST_QUEUE_DEPTH: usize = 64;
const BACKPRESSURE_LOG_EVERY: u64 = 256;

#[derive(Clone, Copy)]
struct ProxyConfig {
    http1_pool_size: usize,
    queue_depth: usize,
}

impl ProxyConfig {
    fn from_env() -> Self {
        Self {
            http1_pool_size: read_env_usize("FROK_HTTP1_POOL_SIZE", DEFAULT_HTTP1_POOL_SIZE),
            queue_depth: read_env_usize("FROK_PROXY_QUEUE_DEPTH", DEFAULT_REQUEST_QUEUE_DEPTH),
        }
    }
}

fn read_env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

impl HttpProxy {
    pub fn new() -> Self {
        Self {
            workers: RwLock::new(HashMap::new()),
            config: ProxyConfig::from_env(),
        }
    }

    pub async fn forward_streaming(
        &self,
        local_addr: &str,
        mode: IngressMode,
        start: HttpRequestStart,
        body_stream: BodyStream,
    ) -> anyhow::Result<ResponseHandle> {
        if mode == IngressMode::Tcp {
            return Err(anyhow::anyhow!("tcp mode not supported for http proxy"));
        }
        let pool = self.get_pool(local_addr, mode).await;
        let (resp_tx, resp_rx) = oneshot::channel();
        let job = RequestJob {
            start,
            mode,
            body_stream,
            resp_tx,
        };

        pool.send(job).await?;

        resp_rx
            .await
            .map_err(|_| anyhow::anyhow!("proxy response dropped"))?
    }

    async fn get_pool(&self, local_addr: &str, mode: IngressMode) -> Arc<WorkerPool> {
        let key = WorkerKey::new(local_addr, mode);
        {
            let guard = self.workers.read().await;
            if let Some(pool) = guard.get(&key) {
                return pool.clone();
            }
        }

        let mut guard = self.workers.write().await;
        if let Some(pool) = guard.get(&key) {
            return pool.clone();
        }

        let pool = WorkerPool::new(local_addr.to_string(), mode, self.config);
        guard.insert(key, pool.clone());
        pool
    }
}

pub struct ResponseHandle {
    pub request_id: u64,
    pub response: hyper::Response<Incoming>,
    pub done: oneshot::Sender<()>,
}

struct RequestJob {
    start: HttpRequestStart,
    mode: IngressMode,
    body_stream: BodyStream,
    resp_tx: oneshot::Sender<anyhow::Result<ResponseHandle>>,
}

type BoxBytesBody = UnsyncBoxBody<Bytes, Infallible>;

struct WorkerPool {
    local_addr: String,
    mode: IngressMode,
    workers: Vec<mpsc::Sender<RequestJob>>,
    next: AtomicUsize,
    backpressure: AtomicU64,
}

impl WorkerPool {
    fn new(local_addr: String, mode: IngressMode, config: ProxyConfig) -> Arc<Self> {
        let worker_count = match mode {
            IngressMode::Http1 => config.http1_pool_size.max(1),
            IngressMode::Http2 => 1,
            IngressMode::Tcp => 0,
        };

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (tx, rx) = mpsc::channel(config.queue_depth);
            match mode {
                IngressMode::Http1 => {
                    tokio::spawn(http1_worker_loop(local_addr.clone(), rx));
                }
                IngressMode::Http2 => {
                    tokio::spawn(http2_worker_loop(local_addr.clone(), rx));
                }
                IngressMode::Tcp => {}
            }
            workers.push(tx);
        }

        Arc::new(Self {
            local_addr,
            mode,
            workers,
            next: AtomicUsize::new(0),
            backpressure: AtomicU64::new(0),
        })
    }

    async fn send(&self, job: RequestJob) -> anyhow::Result<()> {
        if self.workers.is_empty() {
            return Err(anyhow::anyhow!("proxy pool unavailable"));
        }

        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        let sender = &self.workers[idx];
        if sender.capacity() == 0 {
            self.note_backpressure();
        }
        sender
            .send(job)
            .await
            .map_err(|_| anyhow::anyhow!("proxy worker closed"))?;
        Ok(())
    }

    fn note_backpressure(&self) {
        let count = self.backpressure.fetch_add(1, Ordering::Relaxed) + 1;
        if count % BACKPRESSURE_LOG_EVERY == 0 {
            warn!(
                mode = ?self.mode,
                addr = %self.local_addr,
                count,
                "proxy request queue saturated"
            );
        }
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct WorkerKey {
    addr: String,
    mode: IngressMode,
}

impl WorkerKey {
    fn new(addr: &str, mode: IngressMode) -> Self {
        Self {
            addr: addr.to_string(),
            mode,
        }
    }
}

async fn http1_worker_loop(local_addr: String, mut rx: mpsc::Receiver<RequestJob>) {
    let mut sender: Option<http1::SendRequest<BoxBytesBody>> = None;
    let mut conn_task: Option<tokio::task::JoinHandle<()>> = None;

    while let Some(job) = rx.recv().await {
        if sender.as_ref().map(|send| send.is_closed()).unwrap_or(true) {
            if let Err(err) =
                ensure_http1_connection(&local_addr, &mut sender, &mut conn_task).await
            {
                let _ = job.resp_tx.send(Err(err));
                continue;
            }
        }

        if let Some(send) = sender.as_mut() {
            if handle_http1_job(&local_addr, send, job).await.is_err() {
                sender.take();
                if let Some(task) = conn_task.take() {
                    task.abort();
                }
            }
        }
    }

    if let Some(task) = conn_task {
        task.abort();
    }
}

async fn http2_worker_loop(local_addr: String, mut rx: mpsc::Receiver<RequestJob>) {
    let mut sender: Option<http2::SendRequest<BoxBytesBody>> = None;
    let mut conn_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut in_flight = FuturesUnordered::new();

    loop {
        tokio::select! {
            Some(job) = rx.recv() => {
                if sender
                    .as_ref()
                    .map(|send| send.is_closed())
                    .unwrap_or(true)
                {
                    if let Err(err) =
                        ensure_http2_connection(&local_addr, &mut sender, &mut conn_task).await
                    {
                        let _ = job.resp_tx.send(Err(err));
                        continue;
                    }
                }

                if let Some(send) = sender.as_ref() {
                    let local = local_addr.clone();
                    in_flight.push(handle_http2_job(local, send.clone(), job));
                }
            }
            Some(_) = in_flight.next() => {}
            else => break,
        }
    }

    while in_flight.next().await.is_some() {}

    if let Some(task) = conn_task {
        task.abort();
    }
}

async fn ensure_http1_connection(
    local_addr: &str,
    sender: &mut Option<http1::SendRequest<BoxBytesBody>>,
    conn_task: &mut Option<tokio::task::JoinHandle<()>>,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(local_addr).await?;
    let io = TokioIo::new(stream);
    let (send, conn) = http1::Builder::new().handshake(io).await?;
    let task = tokio::spawn(async move {
        if let Err(err) = conn.await {
            warn!(error = %err, "local connection error");
        }
    });
    *sender = Some(send);
    *conn_task = Some(task);
    Ok(())
}

async fn ensure_http2_connection(
    local_addr: &str,
    sender: &mut Option<http2::SendRequest<BoxBytesBody>>,
    conn_task: &mut Option<tokio::task::JoinHandle<()>>,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(local_addr).await?;
    let io = TokioIo::new(stream);
    let (send, conn) = http2::Builder::new(TokioExecutor::new())
        .handshake(io)
        .await?;
    let task = tokio::spawn(async move {
        if let Err(err) = conn.await {
            warn!(error = %err, "local connection error");
        }
    });
    *sender = Some(send);
    *conn_task = Some(task);
    Ok(())
}

async fn handle_http1_job(
    local_addr: &str,
    send: &mut http1::SendRequest<BoxBytesBody>,
    job: RequestJob,
) -> Result<(), anyhow::Error> {
    let request_id = job.start.request_id;
    let request = match build_request(local_addr, job.mode, job.start, job.body_stream) {
        Ok(request) => request,
        Err(err) => {
            let _ = job.resp_tx.send(Err(err));
            return Ok(());
        }
    };

    let response = match send.send_request(request).await {
        Ok(response) => response,
        Err(err) => {
            let err = err.to_string();
            let _ = job.resp_tx.send(Err(anyhow::anyhow!(err.clone())));
            return Err(anyhow::anyhow!(err));
        }
    };

    let (done_tx, done_rx) = oneshot::channel();
    let _ = job.resp_tx.send(Ok(ResponseHandle {
        request_id,
        response,
        done: done_tx,
    }));

    let _ = done_rx.await;
    Ok(())
}

async fn handle_http2_job(
    local_addr: String,
    mut send: http2::SendRequest<BoxBytesBody>,
    job: RequestJob,
) {
    let request_id = job.start.request_id;
    let request = match build_request(&local_addr, job.mode, job.start, job.body_stream) {
        Ok(request) => request,
        Err(err) => {
            let _ = job.resp_tx.send(Err(err));
            return;
        }
    };

    let response = match send.send_request(request).await {
        Ok(response) => response,
        Err(err) => {
            let _ = job.resp_tx.send(Err(anyhow::anyhow!(err)));
            return;
        }
    };

    let (done_tx, done_rx) = oneshot::channel();
    let _ = job.resp_tx.send(Ok(ResponseHandle {
        request_id,
        response,
        done: done_tx,
    }));

    let _ = done_rx.await;
}

fn build_request(
    local_addr: &str,
    mode: IngressMode,
    start: HttpRequestStart,
    body_stream: BodyStream,
) -> anyhow::Result<Request<BoxBytesBody>> {
    let method = start.method.parse::<Method>().unwrap_or(Method::GET);
    let uri = if mode == IngressMode::Http2 {
        http::Uri::builder()
            .scheme("http")
            .authority(local_addr)
            .path_and_query(start.path.as_str())
            .build()
            .unwrap_or_else(|_| http::Uri::from_static("/"))
    } else {
        start
            .path
            .parse()
            .unwrap_or_else(|_| http::Uri::from_static("/"))
    };

    let stream = body_stream.map(|bytes| Ok(Frame::data(bytes)));
    let body = StreamBody::new(stream).boxed_unsync();

    let mut builder = Request::builder().method(method).uri(uri);
    if mode == IngressMode::Http2 {
        builder = builder.version(Version::HTTP_2);
    }
    let mut forwarded_host = None;
    for header in &start.headers {
        if header_bytes_eq(&header.name, "host") {
            forwarded_host = Some(header.value.clone());
            continue;
        }
        if header_bytes_eq(&header.name, "te") {
            if !(mode == IngressMode::Http2 && header_bytes_eq(&header.value, "trailers")) {
                continue;
            }
        } else if is_hop_header_bytes(&header.name)
            || header_bytes_eq(&header.name, "content-length")
        {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(header.name.as_bytes()),
            hyper::header::HeaderValue::from_bytes(header.value.as_bytes()),
        ) {
            builder = builder.header(name, value);
        }
    }

    if let Some(original_host) = forwarded_host {
        if let Ok(value) = hyper::header::HeaderValue::from_bytes(original_host.as_bytes()) {
            builder = builder.header("x-forwarded-host", value);
        }
    }
    builder = builder.header("host", local_addr);

    Ok(builder.body(body)?)
}
