use crate::ingress::admission::AdmissionController;
use crate::ingress::core::{ResolveError, resolve_route_and_peer};
use crate::ingress::runtime::{IngressRuntime, RouteBinding, RouteIngressInfo};
use crate::state::EdgeState;
use async_trait::async_trait;
use bytes::BytesMut;
use frok_common::normalize_host;
use frok_common::tcp::TcpStreamEvent;
use frok_protocol::{
    ByteBuf, IngressMode, MAX_BODY_CHUNK, ServerCommands, TcpStreamData, TcpStreamOpen,
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

struct TcpListenerHandle {
    port: u16,
    token: CancellationToken,
    task: JoinHandle<()>,
}

pub struct TcpIngressManager {
    bind_ip: IpAddr,
    admission: AdmissionController,
    token: CancellationToken,
    listeners: Mutex<HashMap<String, TcpListenerHandle>>,
}

impl TcpIngressManager {
    pub fn new(
        bind_ip: IpAddr,
        admission: AdmissionController,
        token: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            bind_ip,
            admission,
            token,
            listeners: Mutex::new(HashMap::new()),
        })
    }

    async fn register_listener(
        &self,
        hostname: String,
        state: Arc<EdgeState>,
    ) -> anyhow::Result<u16> {
        let host = normalize_host(&hostname);
        {
            let guard = self.listeners.lock().await;
            if let Some(handle) = guard.get(host.as_ref()) {
                return Ok(handle.port);
            }
        }

        let addr = SocketAddr::new(self.bind_ip, 0);
        let listener = TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();

        let mut guard = self.listeners.lock().await;
        if let Some(handle) = guard.get(host.as_ref()) {
            return Ok(handle.port);
        }

        let token = CancellationToken::new();
        let task = spawn_listener(
            listener,
            host.clone().into_owned(),
            state,
            token.clone(),
            self.token.clone(),
            self.admission.clone(),
        );
        guard.insert(host.into_owned(), TcpListenerHandle { port, token, task });

        info!(host = %hostname, port, "tcp ingress registered");
        Ok(port)
    }

    async fn unregister_listener(&self, hostname: &str) {
        let host = normalize_host(hostname);
        let handle = {
            let mut guard = self.listeners.lock().await;
            guard.remove(host.as_ref())
        };

        if let Some(handle) = handle {
            handle.token.cancel();
            let _ = handle.task.await;
            info!(host = %host, "tcp ingress unregistered");
        }
    }
}

#[async_trait]
impl IngressRuntime for TcpIngressManager {
    fn supports(&self, mode: IngressMode) -> bool {
        mode == IngressMode::Tcp
    }

    async fn start(&self, _state: Arc<EdgeState>) -> anyhow::Result<()> {
        Ok(())
    }

    async fn register_route(
        &self,
        binding: RouteBinding,
        state: Arc<EdgeState>,
    ) -> anyhow::Result<RouteIngressInfo> {
        if binding.mode != IngressMode::Tcp {
            anyhow::bail!(
                "tcp ingress cannot register route in {} mode",
                binding.mode.label()
            );
        }

        let host = binding.hostname.clone();
        let port = self.register_listener(binding.hostname, state).await?;
        debug!(
            hostname = %host,
            peer_id = %binding.peer_id,
            mode = %binding.mode.label(),
            port,
            "tcp ingress route registered"
        );
        Ok(RouteIngressInfo {
            mode: IngressMode::Tcp,
            public_port: Some(port),
        })
    }

    async fn unregister_route(&self, hostname: &str) -> anyhow::Result<()> {
        self.unregister_listener(hostname).await;
        Ok(())
    }

    async fn shutdown(&self) -> anyhow::Result<()> {
        self.token.cancel();
        let handles = {
            let mut guard = self.listeners.lock().await;
            guard.drain().map(|(_, handle)| handle).collect::<Vec<_>>()
        };

        for handle in handles {
            handle.token.cancel();
            let _ = handle.task.await;
        }
        Ok(())
    }
}

fn spawn_listener(
    listener: TcpListener,
    hostname: String,
    state: Arc<EdgeState>,
    token: CancellationToken,
    global_token: CancellationToken,
    admission: AdmissionController,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = global_token.cancelled() => break,
                accept = listener.accept() => {
                    let (stream, _) = match accept {
                        Ok(pair) => pair,
                        Err(err) => {
                            warn!(host = %hostname, error = %err, "tcp accept error");
                            continue;
                        }
                    };
                    let connection_permit = match admission.acquire_connection().await {
                        Ok(permit) => permit,
                        Err(err) => {
                            warn!(host = %hostname, error = %err, "tcp connection admission unavailable");
                            break;
                        }
                    };
                    let state = state.clone();
                    let hostname = hostname.clone();
                    let admission = admission.clone();
                    tokio::spawn(async move {
                        let _connection_permit = connection_permit;
                        let _inflight_permit = match admission.acquire_inflight().await {
                            Ok(permit) => permit,
                            Err(err) => {
                                warn!(host = %hostname, error = %err, "tcp inflight admission unavailable");
                                return;
                            }
                        };
                        handle_connection(hostname, stream, state).await;
                    });
                }
            }
        }
    })
}

async fn handle_connection(hostname: String, stream: TcpStream, state: Arc<EdgeState>) {
    let resolved = match resolve_route_and_peer(&state, &hostname, IngressMode::Tcp).await {
        Ok(resolved) => resolved,
        Err(ResolveError::MissingRoute) => {
            debug!(host = %hostname, "tcp connection with no route");
            return;
        }
        Err(ResolveError::ModeMismatch { .. }) => {
            warn!(host = %hostname, "tcp connection for non-tcp route");
            return;
        }
        Err(ResolveError::PeerOffline) => {
            warn!(host = %hostname, "tcp peer offline");
            return;
        }
    };

    let peer = resolved.peer;

    let (stream_id, mut rx) = state.tcp_streams.reserve(peer.id).await;
    if let Err(err) = peer
        .tcp_stream_open(TcpStreamOpen {
            stream_id,
            hostname: hostname.clone(),
        })
        .await
    {
        error!(peer_id = %peer.id, error = %err, "failed to open tcp stream");
        state.tcp_streams.close(stream_id).await;
        return;
    }

    let (mut reader, mut writer) = stream.into_split();
    let write_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                TcpStreamEvent::Data(bytes) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                TcpStreamEvent::End => break,
            }
        }
        let _ = writer.shutdown().await;
    });

    let peer_reader = peer.clone();
    let state_reader = state.clone();
    let read_task = tokio::spawn(async move {
        let mut buf = BytesMut::with_capacity(MAX_BODY_CHUNK);
        loop {
            buf.clear();
            let n = match reader.read_buf(&mut buf).await {
                Ok(n) => n,
                Err(_) => 0,
            };
            if n == 0 {
                let _ = peer_reader
                    .tcp_stream_data(TcpStreamData {
                        stream_id,
                        chunk: ByteBuf::empty(),
                        end: true,
                    })
                    .await;
                break;
            }
            let chunk = buf.split_to(n).freeze();
            if peer_reader
                .tcp_stream_data(TcpStreamData {
                    stream_id,
                    chunk: ByteBuf::from(chunk),
                    end: false,
                })
                .await
                .is_err()
            {
                break;
            }
        }
        state_reader.tcp_streams.close(stream_id).await;
    });

    let _ = tokio::join!(write_task, read_task);
}
