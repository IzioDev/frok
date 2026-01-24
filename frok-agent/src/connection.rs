use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use bytes::Bytes;
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use frok_common::normalize_host;
use frok_common::wire::read_wire_message;

use frok_protocol::{ClientCommands, ServerMessage, WireMessage, WireSender};

use crate::auth::{AgentAuthConfig, AuthTracker};
use crate::build_info;
use crate::client_store::ClientStore;
use crate::host::normalize_route_name;
use crate::http_control::{BodyQueue, handle_request_body, handle_request_start};
use crate::proxy::HttpProxy;
use crate::request_streams::accept_request_streams;
use crate::route_spec::{RouteSpec, format_route};
use crate::routes::{format_public_url, restore_routes};
use crate::tcp_streams::TcpStreams;
use crate::tcp_tunnel::{handle_tcp_data, handle_tcp_open};
use crate::ui::{AppState, AuthStatus, RouteEntry, RouteEvent, RouteStatus, RouteView};

pub(crate) type InflightMap = Arc<dashmap::DashMap<u64, Arc<BodyQueue>>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const OPEN_STREAM_TIMEOUT: Duration = Duration::from_secs(10);
const AUTH_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) struct AgentSender {
    send: mpsc::Sender<Bytes>,
    backpressure: AtomicU64,
}

impl AgentSender {
    fn new(send: SendStream) -> Self {
        let (tx, mut rx) = mpsc::channel::<Bytes>(256);
        tokio::spawn(async move {
            let mut send = send;
            while let Some(payload) = rx.recv().await {
                if let Err(err) = send.write_all(&payload).await {
                    warn!(error = %err, "agent send failed");
                    break;
                }
            }
            let _ = send.finish();
        });
        Self {
            send: tx,
            backpressure: AtomicU64::new(0),
        }
    }

    fn note_backpressure(&self) {
        let count = self.backpressure.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 256 == 0 {
            warn!(count, "agent send queue saturated");
        }
    }
}

#[async_trait]
impl WireSender for AgentSender {
    type Error = anyhow::Error;

    async fn send_wire(&self, message: WireMessage) -> Result<(), Self::Error> {
        let frame = frok_protocol::encode_frame(&message)?;
        if self.send.capacity() == 0 {
            self.note_backpressure();
        }
        self.send
            .send(Bytes::from(frame))
            .await
            .map_err(|_| anyhow::anyhow!("agent send queue closed"))?;
        Ok(())
    }
}

pub(crate) struct SenderRouter {
    inner: RwLock<Option<Arc<AgentSender>>>,
}

impl SenderRouter {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(None),
        })
    }

    pub(crate) async fn set_sender(&self, sender: Option<Arc<AgentSender>>) {
        let mut guard = self.inner.write().await;
        *guard = sender;
    }

    pub(crate) async fn is_connected(&self) -> bool {
        self.inner.read().await.is_some()
    }
}

#[async_trait]
impl WireSender for SenderRouter {
    type Error = anyhow::Error;

    async fn send_wire(&self, message: WireMessage) -> Result<(), Self::Error> {
        let guard = self.inner.read().await;
        let sender = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("not connected"))?;
        sender.send_wire(message).await
    }
}

struct ConnectionHandle {
    connection: Connection,
    token: CancellationToken,
    read_handle: JoinHandle<()>,
    stream_handle: JoinHandle<()>,
}

pub(crate) struct ConnectionManager {
    endpoint_v4: Option<Endpoint>,
    endpoint_v6: Option<Endpoint>,
    edge_host: String,
    edge_port: u16,
    edge_addrs: Vec<SocketAddr>,
    public_domain: Arc<String>,
    state: Arc<AppState>,
    sender: Arc<SenderRouter>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    inflight: InflightMap,
    proxy: Arc<HttpProxy>,
    tcp_streams: Arc<TcpStreams>,
    store: Arc<Mutex<ClientStore>>,
    auth_config: AgentAuthConfig,
    auth_tracker: Arc<AuthTracker>,
    app_token: CancellationToken,
    current: Mutex<Option<ConnectionHandle>>,
    connecting: Mutex<bool>,
}

impl ConnectionManager {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        endpoint_v4: Option<Endpoint>,
        endpoint_v6: Option<Endpoint>,
        edge_host: String,
        edge_port: u16,
        edge_addrs: Vec<SocketAddr>,
        public_domain: Arc<String>,
        state: Arc<AppState>,
        sender: Arc<SenderRouter>,
        active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
        pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
        inflight: InflightMap,
        proxy: Arc<HttpProxy>,
        tcp_streams: Arc<TcpStreams>,
        store: Arc<Mutex<ClientStore>>,
        auth_config: AgentAuthConfig,
        auth_tracker: Arc<AuthTracker>,
        app_token: CancellationToken,
    ) -> Self {
        Self {
            endpoint_v4,
            endpoint_v6,
            edge_host,
            edge_port,
            edge_addrs,
            public_domain,
            state,
            sender,
            active_routes,
            pending_routes,
            inflight,
            proxy,
            tcp_streams,
            store,
            auth_config,
            auth_tracker,
            app_token,
            current: Mutex::new(None),
            connecting: Mutex::new(false),
        }
    }

    pub(crate) async fn connect(&self, token: CancellationToken) -> Result<()> {
        if token.is_cancelled() {
            bail!("cancelled");
        }
        {
            let mut guard = self.connecting.lock().await;
            if *guard {
                self.state.log_info("connect already in progress").await;
                return Ok(());
            }
            *guard = true;
        }

        let result = self.connect_inner(token.clone()).await;
        if result.is_err() {
            self.state.set_status("disconnected").await;
        }
        let mut guard = self.connecting.lock().await;
        *guard = false;
        result
    }

    pub(crate) async fn is_connected(&self) -> bool {
        self.sender.is_connected().await
    }

    async fn connect_inner(&self, token: CancellationToken) -> Result<()> {
        if token.is_cancelled() {
            bail!("cancelled");
        }
        if self.sender.is_connected().await {
            self.state.log_info("already connected").await;
            return Ok(());
        }

        if let Some(handle) = self.take_current().await {
            self.teardown_handle(handle).await;
        }

        self.sender.set_sender(None).await;
        self.auth_tracker.mark_unauthenticated();
        self.auth_tracker.reset_hello_sent();
        self.state.set_edge_build(None).await;
        self.state
            .set_status(format!("connecting: {}:{}", self.edge_host, self.edge_port))
            .await;
        self.state.set_auth_status(AuthStatus::Pending).await;
        self.state.set_auth_url(None).await;
        self.state
            .log_info(format!(
                "dialing edge {}:{} ({} resolved addrs)",
                self.edge_host,
                self.edge_port,
                self.edge_addrs.len()
            ))
            .await;

        if self.edge_addrs.is_empty() {
            bail!(
                "no resolved addresses for {}:{}",
                self.edge_host,
                self.edge_port
            );
        }

        let mut last_err = None;
        let mut connection = None;
        for addr in &self.edge_addrs {
            let endpoint = match addr {
                SocketAddr::V4(_) => self.endpoint_v4.as_ref(),
                SocketAddr::V6(_) => self.endpoint_v6.as_ref(),
            };

            let Some(endpoint) = endpoint else {
                continue;
            };

            let connecting = endpoint
                .connect(*addr, &self.edge_host)
                .context("start connect")?;
            let connect_result = tokio::select! {
                _ = token.cancelled() => {
                    bail!("cancelled");
                }
                result = tokio::time::timeout(CONNECT_TIMEOUT, connecting) => result,
            };
            match connect_result {
                Ok(result) => match result {
                    Ok(conn) => {
                        connection = Some(conn);
                        break;
                    }
                    Err(err) => {
                        last_err = Some(err.into());
                        continue;
                    }
                },
                Err(_) => {
                    last_err = Some(anyhow::anyhow!(
                        "connect timed out after {}s ({})",
                        CONNECT_TIMEOUT.as_secs(),
                        addr
                    ));
                    continue;
                }
            }
        }

        let connection = match connection {
            Some(conn) => conn,
            None => {
                self.state.set_status("disconnected").await;
                let err = last_err.unwrap_or_else(|| {
                    anyhow::anyhow!("unable to connect to any resolved address")
                });
                return Err(err);
            }
        };

        self.state.log_info("opening control stream to edge").await;
        let (send, mut recv) = match tokio::select! {
            _ = token.cancelled() => {
                connection.close(0u32.into(), b"cancelled");
                bail!("cancelled");
            }
            result = tokio::time::timeout(OPEN_STREAM_TIMEOUT, connection.open_bi()) => result,
        } {
            Ok(result) => result.context("open control stream")?,
            Err(_) => {
                self.state.set_status("disconnected").await;
                connection.close(0u32.into(), b"control stream timeout");
                bail!(
                    "control stream open timed out after {}s",
                    OPEN_STREAM_TIMEOUT.as_secs()
                );
            }
        };
        let sender = Arc::new(AgentSender::new(send));

        if !self.auth_tracker.hello_sent() {
            self.state.log_info("sending hello (pre-auth)").await;
            sender
                .hello(
                    self.auth_config.agent_label.clone(),
                    build_info::local_build_info(),
                )
                .await?;
            self.auth_tracker.mark_hello_sent();
        }

        self.state.log_info("waiting for auth challenge").await;
        let handshake = tokio::select! {
            _ = token.cancelled() => {
                connection.close(0u32.into(), b"cancelled");
                bail!("cancelled");
            }
            result = tokio::time::timeout(
                AUTH_HANDSHAKE_TIMEOUT,
                crate::auth::perform_handshake(
                    &mut recv,
                    sender.clone(),
                    self.state.clone(),
                    self.auth_config.clone(),
                    self.auth_tracker.clone(),
                ),
            ) => result,
        };

        if let Err(err) = match handshake {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!(
                "auth handshake timed out after {}s",
                AUTH_HANDSHAKE_TIMEOUT.as_secs()
            )),
        } {
            self.state.log_error(format!("auth failed: {err}")).await;
            self.state
                .set_auth_status(AuthStatus::Failed {
                    reason: err.to_string(),
                })
                .await;
            self.state.set_status("disconnected").await;
            self.sender.set_sender(None).await;
            connection.close(0u32.into(), b"auth failed");
            return Err(err);
        }

        if token.is_cancelled() {
            connection.close(0u32.into(), b"cancelled");
            bail!("cancelled");
        }

        self.sender.set_sender(Some(sender.clone())).await;
        self.state
            .set_status(format!(
                "connected: {} ({})",
                connection.remote_address(),
                self.edge_host
            ))
            .await;
        self.state.log_info("connected to edge").await;

        let conn_token = self.app_token.child_token();
        let read_handle = tokio::spawn(read_loop(
            recv,
            sender.clone(),
            self.sender.clone(),
            self.state.clone(),
            self.active_routes.clone(),
            self.pending_routes.clone(),
            self.inflight.clone(),
            self.proxy.clone(),
            self.tcp_streams.clone(),
            self.store.clone(),
            self.auth_config.clone(),
            self.auth_tracker.clone(),
            conn_token.clone(),
        ));

        let stream_handle = tokio::spawn(accept_request_streams(
            connection.clone(),
            self.state.clone(),
            self.active_routes.clone(),
            self.proxy.clone(),
            conn_token.clone(),
        ));

        restore_routes(
            self.sender.clone(),
            self.state.clone(),
            self.pending_routes.clone(),
            self.store.clone(),
            self.public_domain.clone(),
        )
        .await;

        let handle = ConnectionHandle {
            connection,
            token: conn_token,
            read_handle,
            stream_handle,
        };
        let mut guard = self.current.lock().await;
        *guard = Some(handle);

        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        if let Some(handle) = self.take_current().await {
            self.teardown_handle(handle).await;
        }
        self.sender.set_sender(None).await;
    }

    async fn take_current(&self) -> Option<ConnectionHandle> {
        let mut guard = self.current.lock().await;
        guard.take()
    }

    async fn teardown_handle(&self, handle: ConnectionHandle) {
        handle.token.cancel();
        handle.connection.close(0u32.into(), b"disconnect");
        let mut read_handle = handle.read_handle;
        let mut stream_handle = handle.stream_handle;

        let read_timeout = sleep(Duration::from_millis(300));
        tokio::pin!(read_timeout);
        tokio::select! {
            _ = &mut read_handle => {}
            _ = &mut read_timeout => {
                read_handle.abort();
                let _ = read_handle.await;
            }
        }

        let stream_timeout = sleep(Duration::from_millis(300));
        tokio::pin!(stream_timeout);
        tokio::select! {
            _ = &mut stream_handle => {}
            _ = &mut stream_timeout => {
                stream_handle.abort();
                let _ = stream_handle.await;
            }
        }
    }
}

pub(crate) async fn connection_supervisor(
    connection_manager: Arc<ConnectionManager>,
    state: Arc<AppState>,
    token: CancellationToken,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        if token.is_cancelled() {
            break;
        }

        if !connection_manager.is_connected().await {
            match connection_manager.connect(token.clone()).await {
                Ok(()) => {
                    backoff = Duration::from_secs(1);
                }
                Err(err) => {
                    if token.is_cancelled() {
                        break;
                    }
                    state
                        .log_warn(format!(
                            "disconnected, retrying in {}s: {err}",
                            backoff.as_secs()
                        ))
                        .await;
                    let sleep = sleep(backoff);
                    tokio::select! {
                        _ = token.cancelled() => break,
                        _ = sleep => {}
                    }
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                }
            }
        }

        let idle = sleep(Duration::from_secs(1));
        tokio::select! {
            _ = token.cancelled() => break,
            _ = idle => {}
        }
    }
}

async fn read_loop(
    mut recv: RecvStream,
    sender: Arc<AgentSender>,
    sender_router: Arc<SenderRouter>,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    inflight: InflightMap,
    proxy: Arc<HttpProxy>,
    tcp_streams: Arc<TcpStreams>,
    store: Arc<Mutex<ClientStore>>,
    auth_config: AgentAuthConfig,
    auth_tracker: Arc<AuthTracker>,
    token: CancellationToken,
) {
    let mut recv_buf = Vec::new();
    loop {
        let message = tokio::select! {
            _ = token.cancelled() => break,
            message = read_wire_message(&mut recv, &mut recv_buf) => message,
        };

        let message = match message {
            Ok(message) => message,
            Err(err) => {
                state.log_error(format!("receive error: {err}")).await;
                state.set_status("disconnected").await;
                break;
            }
        };

        match message {
            WireMessage::Server(message) => {
                handle_server_message(
                    message,
                    sender.clone(),
                    state.clone(),
                    active_routes.clone(),
                    pending_routes.clone(),
                    inflight.clone(),
                    proxy.clone(),
                    tcp_streams.clone(),
                    store.clone(),
                    auth_config.clone(),
                    auth_tracker.clone(),
                )
                .await;
            }
            WireMessage::Client(_) => {
                state.log_warn("unexpected client message").await;
            }
        }
    }

    state.set_status("disconnected").await;
    state.set_auth_status(AuthStatus::Unknown).await;
    state.set_auth_url(None).await;
    state.set_edge_build(None).await;
    auth_tracker.mark_unauthenticated();
    auth_tracker.reset_hello_sent();
    sender_router.set_sender(None).await;
}

async fn handle_server_message(
    message: ServerMessage,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    inflight: InflightMap,
    proxy: Arc<HttpProxy>,
    tcp_streams: Arc<TcpStreams>,
    store: Arc<Mutex<ClientStore>>,
    auth_config: AgentAuthConfig,
    auth_tracker: Arc<AuthTracker>,
) {
    match message {
        ServerMessage::AuthChallenge { nonce, oidc } => {
            if let Err(err) = crate::auth::handle_auth_challenge(
                nonce,
                oidc,
                sender.clone(),
                state.clone(),
                auth_config.clone(),
            )
            .await
            {
                state
                    .log_error(format!("auth challenge failed: {err}"))
                    .await;
            }
        }
        ServerMessage::AuthOk {
            subject,
            session_id,
        } => {
            state
                .log_info(format!("auth ok: {subject} (session {session_id})"))
                .await;
            state
                .set_auth_status(AuthStatus::Authenticated {
                    subject: subject.clone(),
                })
                .await;
            state.set_auth_url(None).await;
            auth_tracker.mark_authenticated();
            if !auth_tracker.hello_sent() {
                if let Err(err) = sender
                    .hello(
                        auth_config.agent_label.clone(),
                        build_info::local_build_info(),
                    )
                    .await
                {
                    state.log_error(format!("hello send failed: {err}")).await;
                } else {
                    auth_tracker.mark_hello_sent();
                }
            }
        }
        ServerMessage::AuthErr { reason } => {
            state.log_error(format!("auth failed: {reason}")).await;
            state
                .set_auth_status(AuthStatus::Failed {
                    reason: reason.clone(),
                })
                .await;
            state.set_auth_url(None).await;
            auth_tracker.mark_unauthenticated();
            auth_tracker.reset_hello_sent();
        }
        ServerMessage::HelloAck { session_id, build } => {
            state
                .log_info(format!(
                    "hello ack: session {session_id} ({})",
                    build_info::format_build(&build)
                ))
                .await;
            let local_build = build_info::local_build_info();
            if build.version != local_build.version {
                state
                    .log_warn(format!(
                        "edge version mismatch: local v{} vs edge v{}",
                        local_build.version, build.version
                    ))
                    .await;
            }
            state.set_edge_build(Some(build)).await;
        }
        ServerMessage::RegisterOk { hostname, ingress } => {
            let host = normalize_host(&hostname);
            let name = normalize_route_name(host.as_ref()).into_owned();
            let route = {
                let mut guard = pending_routes.lock().await;
                guard.remove(host.as_ref())
            };

            if let Some(route) = route {
                let host_owned = host.into_owned();
                let route = RouteSpec {
                    local_addr: route.local_addr,
                    mode: ingress.mode,
                };
                {
                    let mut guard = active_routes.write().await;
                    guard.insert(host_owned.clone(), route.clone());
                }
                let entry = RouteEntry {
                    name: name.clone(),
                    public_url: format_public_url(&host_owned, ingress.mode, ingress.public_port),
                    local_addr: route.local_addr.clone(),
                    mode: ingress.mode,
                    status: RouteStatus::Live,
                };
                state.register_route(entry).await;
                let view = RouteView {
                    name: name.clone(),
                    full_host: host_owned.clone(),
                    ingress: ingress.clone(),
                };
                state.publish_route_event(RouteEvent::Registered(view));
                state
                    .log_info(format!(
                        "registered: {host_owned} -> {}",
                        format_route(&route, &ingress)
                    ))
                    .await;

                {
                    let result = {
                        let mut guard = store.lock().await;
                        guard.upsert(name.clone(), route.clone())
                    };
                    if let Err(err) = result {
                        state
                            .log_error(format!("route store update failed: {err}"))
                            .await;
                    }
                }
            } else {
                state
                    .log_warn(format!("register ok for unknown host: {host}"))
                    .await;
            }
        }
        ServerMessage::RegisterErr { hostname, reason } => {
            let host = normalize_host(&hostname);
            let name = normalize_route_name(host.as_ref()).into_owned();
            {
                let mut guard = pending_routes.lock().await;
                guard.remove(host.as_ref());
            }
            let host_owned = host.into_owned();
            state
                .log_warn(format!("register failed: {host_owned} ({reason})"))
                .await;
            if !name.is_empty() {
                state.set_route_status(&name, RouteStatus::Error).await;
            }
            state.publish_route_event(RouteEvent::RegisterFailed {
                name,
                full_host: host_owned,
                reason,
            });
        }
        ServerMessage::Ping { nonce } => {
            if let Err(err) = sender.pong(nonce).await {
                state.log_error(format!("pong send failed: {err}")).await;
            }
        }
        ServerMessage::HttpRequestStart { request } => {
            handle_request_start(
                request,
                sender.clone(),
                state.clone(),
                active_routes.clone(),
                inflight.clone(),
                proxy.clone(),
            )
            .await;
        }
        ServerMessage::HttpRequestBody { body } => {
            handle_request_body(body, inflight, state.clone()).await;
        }
        ServerMessage::TcpStreamOpen { stream } => {
            handle_tcp_open(
                stream,
                sender.clone(),
                state.clone(),
                active_routes.clone(),
                tcp_streams.clone(),
            )
            .await;
        }
        ServerMessage::TcpStreamData { data } => {
            handle_tcp_data(data, tcp_streams.clone(), state.clone()).await;
        }
    }
}
