use crate::ingress::admission::AdmissionController;
use crate::ingress::core::{ResolveError, resolve_route_and_peer};
use crate::ingress::runtime::{IngressRuntime, RouteBinding, RouteIngressInfo};
use crate::state::EdgeState;
use async_stream::stream;
use async_trait::async_trait;
use bytes::Bytes;
use frok_common::http::{header_bytes_eq, is_hop_header_bytes};
use frok_common::normalize_host;
use frok_common::wire::{read_wire_message, write_wire_message};
use http_body::Frame;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Incoming;
use hyper::header::HOST;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use frok_protocol::{
    ByteBuf, ClientMessage, Header, HttpRequestBody, HttpRequestStart, IngressMode, MAX_BODY_CHUNK,
    WireMessage,
};

type BoxBytesBody = BoxBody<Bytes, Infallible>;

const HTTP2_PREFACE_PREFIX: &[u8; 3] = b"PRI";
static HTTP_REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct HttpIngressManager {
    addr: SocketAddr,
    admission: AdmissionController,
    token: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl HttpIngressManager {
    pub fn new(
        addr: SocketAddr,
        admission: AdmissionController,
        token: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            addr,
            admission,
            token,
            task: Mutex::new(None),
        })
    }
}

async fn run_accept_loop(
    addr: SocketAddr,
    admission: AdmissionController,
    token: CancellationToken,
    state: Arc<EdgeState>,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            error!(addr = %addr, error = %err, "failed to bind http listener");
            return;
        }
    };

    info!(addr = %addr, "http ingress listening");
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _) = match accept {
                    Ok(pair) => pair,
                    Err(err) => {
                        warn!(error = %err, "http accept error");
                        continue;
                    }
                };

                let state = state.clone();
                let admission = admission.clone();
                let permit = match admission.acquire_connection().await {
                    Ok(permit) => permit,
                    Err(err) => {
                        warn!(error = %err, "http connection admission unavailable");
                        break;
                    }
                };
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(err) = serve_connection(stream, state, admission, addr).await {
                        warn!(error = %err, "http connection error");
                    }
                });
            }
        }
    }
}

async fn serve_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<EdgeState>,
    admission: AdmissionController,
    addr: SocketAddr,
) -> Result<(), anyhow::Error> {
    let is_h2 = is_http2_preface(&mut stream).await;
    let io = TokioIo::new(stream);
    let service = service_fn(move |req| {
        let state = state.clone();
        let admission = admission.clone();
        async move {
            let _permit = match admission.acquire_inflight().await {
                Ok(permit) => permit,
                Err(err) => {
                    warn!(error = %err, "http inflight admission unavailable");
                    return Ok(simple_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "request admission unavailable",
                    ));
                }
            };
            handle_request(req, state).await
        }
    });

    if is_h2 {
        debug!(addr = %addr, "http2 ingress connection");
        http2::Builder::new(TokioExecutor::new())
            .serve_connection(io, service)
            .await?;
    } else {
        debug!(addr = %addr, "http1 ingress connection");
        http1::Builder::new().serve_connection(io, service).await?;
    }

    Ok(())
}

#[async_trait]
impl IngressRuntime for HttpIngressManager {
    fn supports(&self, mode: IngressMode) -> bool {
        matches!(mode, IngressMode::Http1 | IngressMode::Http2)
    }

    async fn start(&self, state: Arc<EdgeState>) -> anyhow::Result<()> {
        let mut guard = self.task.lock().await;
        if guard.is_some() {
            return Ok(());
        }

        let addr = self.addr;
        let admission = self.admission.clone();
        let token = self.token.clone();
        let task = tokio::spawn(async move {
            run_accept_loop(addr, admission, token, state).await;
        });
        *guard = Some(task);
        Ok(())
    }

    async fn register_route(
        &self,
        binding: RouteBinding,
        _state: Arc<EdgeState>,
    ) -> anyhow::Result<RouteIngressInfo> {
        if !self.supports(binding.mode) {
            anyhow::bail!(
                "http ingress cannot register route in {} mode",
                binding.mode.label()
            );
        }
        debug!(
            hostname = %binding.hostname,
            peer_id = %binding.peer_id,
            mode = %binding.mode.label(),
            "http ingress route registered"
        );

        Ok(RouteIngressInfo {
            mode: binding.mode,
            public_port: Some(self.addr.port()),
        })
    }

    async fn unregister_route(&self, _hostname: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        self.token.cancel();
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
        Ok(())
    }
}

async fn is_http2_preface(stream: &mut tokio::net::TcpStream) -> bool {
    let mut buf = [0u8; 3];
    let deadline = Instant::now() + Duration::from_millis(50);
    loop {
        match stream.peek(&mut buf).await {
            Ok(n) if n >= buf.len() => return buf == *HTTP2_PREFACE_PREFIX,
            Ok(0) => return false,
            Ok(_) => {}
            Err(_) => return false,
        }

        if Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(1)).await;
    }
}

async fn handle_request(
    req: Request<Incoming>,
    state: Arc<EdgeState>,
) -> Result<Response<BoxBytesBody>, Infallible> {
    let request_id = HTTP_REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let host = match extract_host(&req) {
        Some(host) => host,
        None => {
            warn!("http request missing host");
            return Ok(simple_response(StatusCode::BAD_REQUEST, "missing host"));
        }
    };

    let request_mode = match req.version() {
        Version::HTTP_2 => IngressMode::Http2,
        _ => IngressMode::Http1,
    };

    let resolved = match resolve_route_and_peer(&state, host.as_ref(), request_mode).await {
        Ok(resolved) => resolved,
        Err(ResolveError::MissingRoute) => {
            warn!(host = %host, "no route for host");
            return Ok(simple_response(StatusCode::NOT_FOUND, "unknown host"));
        }
        Err(ResolveError::ModeMismatch {
            actual: IngressMode::Tcp,
        }) => {
            warn!(host = %host, "tcp route received http request");
            return Ok(simple_response(
                StatusCode::BAD_REQUEST,
                "route is tcp-only",
            ));
        }
        Err(ResolveError::ModeMismatch { .. }) => {
            warn!(host = %host, "route protocol mismatch");
            return Ok(simple_response(
                StatusCode::UPGRADE_REQUIRED,
                "route protocol mismatch",
            ));
        }
        Err(ResolveError::PeerOffline) => {
            warn!(host = %host, "peer offline");
            return Ok(simple_response(StatusCode::BAD_GATEWAY, "peer offline"));
        }
    };

    let peer = resolved.peer;

    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or("/")
        .to_string();
    let mut headers = Vec::with_capacity(req.headers().len());
    for (name, value) in req.headers().iter() {
        headers.push(Header {
            name: ByteBuf::from(Bytes::copy_from_slice(name.as_str().as_bytes())),
            value: ByteBuf::from(Bytes::copy_from_slice(value.as_bytes())),
        });
    }

    let (mut send, mut recv) = match peer.open_bi().await {
        Ok(pair) => pair,
        Err(err) => {
            error!(peer_id = %peer.id, error = %err, "failed to open stream to peer");
            return Ok(simple_response(StatusCode::BAD_GATEWAY, "forward failed"));
        }
    };

    let request_start = HttpRequestStart {
        request_id,
        method,
        path,
        headers,
    };

    if let Err(err) = write_wire_message(
        &mut send,
        &WireMessage::Server(frok_protocol::ServerMessage::HttpRequestStart {
            request: request_start,
        }),
    )
    .await
    {
        error!(peer_id = %peer.id, error = %err, "failed to forward request");
        return Ok(simple_response(StatusCode::BAD_GATEWAY, "forward failed"));
    }

    let mut body = req.into_body();
    while let Some(frame) = body.frame().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(err) => {
                warn!(error = %err, "failed to read http body");
                return Ok(simple_response(StatusCode::BAD_REQUEST, "invalid body"));
            }
        };

        let data = match frame.into_data() {
            Ok(data) => data,
            Err(_) => continue,
        };

        if let Err(err) = send_body_chunks(&mut send, request_id, data).await {
            error!(peer_id = %peer.id, error = %err, "failed to stream request body");
            return Ok(simple_response(StatusCode::BAD_GATEWAY, "forward failed"));
        }
    }

    if let Err(err) = write_wire_message(
        &mut send,
        &WireMessage::Server(frok_protocol::ServerMessage::HttpRequestBody {
            body: HttpRequestBody {
                request_id,
                chunk: ByteBuf::empty(),
                end: true,
            },
        }),
    )
    .await
    {
        error!(peer_id = %peer.id, error = %err, "failed to finish request body");
        return Ok(simple_response(StatusCode::BAD_GATEWAY, "forward failed"));
    }

    let _ = send.finish();

    let mut recv_buf = Vec::new();
    let response_start = match timeout(
        Duration::from_secs(10),
        read_wire_message(&mut recv, &mut recv_buf),
    )
    .await
    {
        Ok(Ok(WireMessage::Client(ClientMessage::HttpResponseStart { response }))) => response,
        Ok(Ok(_)) => {
            warn!(peer_id = %peer.id, "invalid response start");
            return Ok(simple_response(StatusCode::BAD_GATEWAY, "invalid response"));
        }
        Ok(Err(err)) => {
            error!(peer_id = %peer.id, error = %err, "failed to read response start");
            return Ok(simple_response(StatusCode::BAD_GATEWAY, "invalid response"));
        }
        Err(_) => {
            warn!(peer_id = %peer.id, "response start timed out");
            return Ok(simple_response(StatusCode::GATEWAY_TIMEOUT, "timeout"));
        }
    };

    let mut builder = Response::builder().status(response_start.status);
    for header in response_start.headers {
        if is_hop_header_bytes(&header.name) || header_bytes_eq(&header.name, "content-length") {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            hyper::header::HeaderName::from_bytes(header.name.as_bytes()),
            hyper::header::HeaderValue::from_bytes(header.value.as_bytes()),
        ) {
            builder = builder.header(name, value);
        }
    }

    let body_stream = stream! {
        let mut recv = recv;
        let mut buffer = recv_buf;
        loop {
            let message = match read_wire_message(&mut recv, &mut buffer).await {
                Ok(message) => message,
                Err(_) => break,
            };
            match message {
                WireMessage::Client(ClientMessage::HttpResponseBody { body }) => {
                    let bytes: Bytes = body.chunk.into();
                    if !bytes.is_empty() {
                        yield Ok(Frame::data(bytes));
                    }
                    if body.end {
                        break;
                    }
                }
                _ => {}
            }
        }
    };
    let body = StreamBody::new(body_stream).boxed();

    match builder.body(body) {
        Ok(response) => Ok(response),
        Err(_) => Ok(simple_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid response",
        )),
    }
}

fn extract_host(req: &Request<Incoming>) -> Option<std::borrow::Cow<'_, str>> {
    if let Some(host) = req.uri().host() {
        return Some(normalize_host(host));
    }

    let header = req.headers().get(HOST)?;
    let header = std::str::from_utf8(header.as_bytes()).ok()?;
    let host = header.split(':').next().unwrap_or(header);
    Some(normalize_host(host))
}

fn simple_response(status: StatusCode, message: &'static str) -> Response<BoxBytesBody> {
    let body = Bytes::from_static(message.as_bytes());
    let response = Response::new(Full::new(body).boxed());
    let mut response = response;
    *response.status_mut() = status;
    response
}

async fn send_body_chunks(
    send: &mut quinn::SendStream,
    request_id: u64,
    data: Bytes,
) -> Result<(), anyhow::Error> {
    let mut start = 0;
    while start < data.len() {
        let end = (start + MAX_BODY_CHUNK).min(data.len());
        let chunk = data.slice(start..end);
        write_wire_message(
            send,
            &WireMessage::Server(frok_protocol::ServerMessage::HttpRequestBody {
                body: HttpRequestBody {
                    request_id,
                    chunk: ByteBuf::from(chunk),
                    end: false,
                },
            }),
        )
        .await?;
        start = end;
    }
    Ok(())
}
