use crate::ingress::RouteBinding;
use crate::peer::{PeerHandle, PeerId};
use crate::routes::RegisterResult;
use crate::state::EdgeState;
use crate::store::{IdentityRecord, RouteOwnership};
use bytes::Bytes;
use frok_common::normalize_host;
use frok_common::time::now_ts;
use frok_common::wire::{ReadWireError, read_wire_message};
use frok_protocol::{
    ClientMessage, IngressMode, RegisteredIngress, ServerCommands, TcpStreamData, WireMessage,
};
use quinn::{Connection, Incoming, RecvStream};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

use crate::build_info;
pub fn spawn_incoming(incoming: Incoming, state: Arc<EdgeState>) {
    tokio::spawn(async move {
        match incoming.await {
            Ok(connection) => handle_connection(connection, state).await,
            Err(err) => warn!(error = %err, "error during handshake"),
        }
    });
}

async fn handle_connection(connection: Connection, state: Arc<EdgeState>) {
    let remote = connection.remote_address();
    info!(remote = %remote, "handshake completed");

    match connection.accept_bi().await {
        Ok((send, mut recv)) => {
            let peer = state.peers.insert(remote, connection.clone(), send).await;
            info!(peer_id = %peer.id, remote = %peer.remote, "registered connection");

            let mut auth_state = AuthState::new(state.auth.is_insecure());
            if let Some((nonce, oidc)) = state.auth.make_challenge() {
                auth_state.set_challenge(nonce.clone());
                if let Err(err) = peer.auth_challenge(nonce, oidc).await {
                    error!(peer_id = %peer.id, error = %err, "failed to send auth challenge");
                    return;
                }
            } else if !auth_state.required {
                if let Err(err) = peer.auth_ok("insecure".to_string(), peer.id.as_u64()).await {
                    error!(peer_id = %peer.id, error = %err, "failed to send auth ok");
                    return;
                }
            }

            read_loop(&mut recv, peer.clone(), state.clone(), &mut auth_state).await;

            let removed = state.routes.remove_peer(peer.id).await;
            if !removed.is_empty() {
                for (host, entry) in &removed {
                    debug!(
                        peer_id = %peer.id,
                        hostname = %host,
                        local_addr = %entry.local_addr,
                        mode = ?entry.mode,
                        "route removed"
                    );
                    if let Err(err) = state.ingress.unregister_route(entry.mode, host).await {
                        warn!(
                            peer_id = %peer.id,
                            hostname = %host,
                            error = %err,
                            "failed to unregister ingress route"
                        );
                    }
                }
                info!(peer_id = %peer.id, count = removed.len(), "removed routes for peer");
            }
            state.peers.remove(peer.id).await;
            info!(peer_id = %peer.id, "disconnected");
        }
        Err(err) => {
            warn!(remote = %remote, error = %err, "error opening stream");
        }
    }
}

async fn read_loop(
    recv: &mut RecvStream,
    peer: Arc<PeerHandle>,
    state: Arc<EdgeState>,
    auth_state: &mut AuthState,
) {
    let id = peer.id;
    let mut hello_seen = false;
    let mut payload = Vec::new();
    loop {
        let message = match read_wire_message(recv, &mut payload).await {
            Ok(message) => message,
            Err(ReadWireError::FrameTooLarge { len, .. }) => {
                warn!(peer_id = %id, frame_len = len, "frame too large");
                break;
            }
            Err(err) => {
                warn!(peer_id = %id, error = %err, "receive error");
                break;
            }
        };

        match message {
            WireMessage::Client(message) => {
                if auth_state.required && !auth_state.authenticated {
                    match message {
                        ClientMessage::Auth { method } => {
                            match auth_state
                                .verify_and_bind(peer.clone(), state.clone(), method)
                                .await
                            {
                                Ok(subject) => {
                                    if let Err(err) = peer.auth_ok(subject, peer.id.as_u64()).await
                                    {
                                        error!(
                                            peer_id = %peer.id,
                                            error = %err,
                                            "failed to send auth ok"
                                        );
                                        break;
                                    }
                                }
                                Err(err) => {
                                    let _ = peer.auth_err(err).await;
                                    break;
                                }
                            }
                        }
                        ClientMessage::Hello { .. } => {
                            handle_client_message(
                                peer.clone(),
                                state.clone(),
                                message,
                                &mut hello_seen,
                                auth_state.subject.as_deref(),
                                auth_state.required,
                            )
                            .await;
                        }
                        _ => {
                            let _ = peer.auth_err("auth required").await;
                            break;
                        }
                    }
                    continue;
                } else if state.auth.is_insecure() {
                    if let ClientMessage::Auth { .. } = message {
                        let _ = peer.auth_err("auth not required").await;
                        continue;
                    }
                }

                handle_client_message(
                    peer.clone(),
                    state.clone(),
                    message,
                    &mut hello_seen,
                    auth_state.subject.as_deref(),
                    auth_state.required,
                )
                .await;
            }
            WireMessage::Server(_) => {
                warn!(peer_id = %id, "unexpected server message");
                break;
            }
        }
    }
}

async fn handle_client_message(
    peer: Arc<PeerHandle>,
    state: Arc<EdgeState>,
    message: ClientMessage,
    hello_seen: &mut bool,
    subject: Option<&str>,
    auth_required: bool,
) {
    debug!(peer_id = %peer.id, message = ?message, "received client message");
    match message {
        ClientMessage::Hello { agent, build } => {
            let edge_build = build_info::local_build_info();
            info!(
                peer_id = %peer.id,
                agent = %agent,
                agent_build = %build_info::format_build(&build),
                "agent connected"
            );
            if build.version != edge_build.version {
                warn!(
                    peer_id = %peer.id,
                    agent_version = %build.version,
                    edge_version = %edge_build.version,
                    "agent/edge version mismatch"
                );
            }
            *hello_seen = true;
            if let Err(err) = peer.hello_ack(peer.id.as_u64(), edge_build).await {
                error!(peer_id = %peer.id, error = %err, "failed to send hello ack");
            }
        }
        ClientMessage::Register {
            hostname,
            local_addr,
            mode,
        } => {
            handle_register(
                peer.clone(),
                state.clone(),
                hostname,
                local_addr,
                mode,
                *hello_seen,
                subject,
                auth_required,
            )
            .await;
        }
        ClientMessage::Unregister { hostname } => {
            if !*hello_seen {
                warn!(
                    peer_id = %peer.id,
                    hostname = %hostname,
                    "ignoring unregister before hello"
                );
                return;
            }
            let removed = state.routes.unregister(&hostname, peer.id).await;
            if let Some(entry) = removed {
                if let Err(err) = state.ingress.unregister_route(entry.mode, &hostname).await {
                    warn!(
                        peer_id = %peer.id,
                        hostname = %hostname,
                        error = %err,
                        "failed to unregister ingress route"
                    );
                }
                info!(
                    peer_id = %peer.id,
                    hostname = %hostname,
                    local_addr = %entry.local_addr,
                    removed = true,
                    "unregister request"
                );
            } else {
                info!(
                    peer_id = %peer.id,
                    hostname = %hostname,
                    removed = false,
                    "unregister request"
                );
            }
        }
        ClientMessage::HttpResponseStart { response } => {
            if !*hello_seen {
                warn!(
                    peer_id = %peer.id,
                    request_id = response.request_id,
                    "ignoring http response start before hello"
                );
                return;
            }
            warn!(
                peer_id = %peer.id,
                request_id = response.request_id,
                "ignoring http response start on control stream"
            );
        }
        ClientMessage::HttpResponseBody { body } => {
            if !*hello_seen {
                warn!(
                    peer_id = %peer.id,
                    request_id = body.request_id,
                    "ignoring http response body before hello"
                );
                return;
            }
            warn!(
                peer_id = %peer.id,
                request_id = body.request_id,
                "ignoring http response body on control stream"
            );
        }
        ClientMessage::TcpStreamData { data } => {
            if !*hello_seen {
                warn!(
                    peer_id = %peer.id,
                    stream_id = data.stream_id,
                    "ignoring tcp stream data before hello"
                );
                return;
            }
            handle_tcp_stream_data(state.clone(), peer.id, data).await;
        }
        ClientMessage::Pong { nonce } => {
            debug!(peer_id = %peer.id, nonce, "pong");
        }
        ClientMessage::Auth { .. } => {
            warn!(peer_id = %peer.id, "unexpected auth message");
        }
    }
}

async fn handle_tcp_stream_data(
    state: Arc<EdgeState>,
    peer_id: crate::peer::PeerId,
    data: TcpStreamData,
) {
    let resolved = state
        .tcp_streams
        .send_data(peer_id, data.stream_id, Bytes::from(data.chunk), data.end)
        .await;
    if !resolved {
        warn!(
            peer_id = %peer_id,
            stream_id = data.stream_id,
            "orphan tcp stream data"
        );
    }
}

enum RegisterFailure {
    AlreadyTaken,
    IngressRegistrationFailed(String),
}

async fn handle_register(
    peer: Arc<PeerHandle>,
    state: Arc<EdgeState>,
    hostname: String,
    local_addr: String,
    mode: IngressMode,
    hello_seen: bool,
    subject: Option<&str>,
    auth_required: bool,
) {
    let subject = match resolve_register_subject(hello_seen, auth_required, subject) {
        Ok(subject) => subject,
        Err(reason) => {
            send_register_err(&peer, &hostname, reason).await;
            return;
        }
    };

    if auth_required {
        if let Err(reason) = ensure_route_ownership(&state, peer.id, &hostname, subject) {
            send_register_err(&peer, &hostname, reason).await;
            return;
        }
    }

    match register_route_and_ingress(state, peer.id, &hostname, local_addr, mode, subject).await {
        Ok(ingress) => {
            if let Err(err) = peer.register_ok(hostname, ingress).await {
                error!(peer_id = %peer.id, error = %err, "failed to send register ok");
            }
        }
        Err(RegisterFailure::AlreadyTaken) => {
            send_register_err(&peer, &hostname, "hostname already registered").await;
        }
        Err(RegisterFailure::IngressRegistrationFailed(reason)) => {
            send_register_err(
                &peer,
                &hostname,
                format!("ingress registration failed: {reason}"),
            )
            .await;
        }
    }
}

fn resolve_register_subject<'a>(
    hello_seen: bool,
    auth_required: bool,
    subject: Option<&'a str>,
) -> Result<&'a str, &'static str> {
    if !hello_seen {
        return Err("hello required before register");
    }
    if auth_required && subject.is_none() {
        return Err("auth required before register");
    }
    Ok(subject.unwrap_or("insecure"))
}

fn ensure_route_ownership(
    state: &EdgeState,
    peer_id: PeerId,
    hostname: &str,
    subject: &str,
) -> Result<(), &'static str> {
    let host = normalize_host(hostname).into_owned();
    match state.store.load_route(&host) {
        Ok(Some(route)) if route.subject != subject => Err("hostname owned by another identity"),
        Ok(None) => {
            let registered_at = match now_ts() {
                Ok(ts) => ts,
                Err(err) => {
                    error!(
                        peer_id = %peer_id,
                        error = %err,
                        "failed to read system time"
                    );
                    return Err("failed to read system time");
                }
            };
            let record = RouteOwnership {
                hostname: host.clone(),
                subject: subject.to_string(),
                registered_at,
            };
            if let Err(err) = state.store.insert_route(&record) {
                error!(
                    peer_id = %peer_id,
                    error = %err,
                    "failed to persist route ownership"
                );
                return Err("failed to persist route ownership");
            }
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(err) => {
            error!(
                peer_id = %peer_id,
                error = %err,
                "route ownership lookup failed"
            );
            Err("route ownership lookup failed")
        }
    }
}

async fn register_route_and_ingress(
    state: Arc<EdgeState>,
    peer_id: PeerId,
    hostname: &str,
    local_addr: String,
    mode: IngressMode,
    subject: &str,
) -> Result<RegisteredIngress, RegisterFailure> {
    let result = state
        .routes
        .register(
            hostname.to_string(),
            subject.to_string(),
            peer_id,
            local_addr,
            mode,
        )
        .await;
    match result {
        RegisterResult::Ok => {
            let ingress = match state
                .ingress
                .register_route(
                    RouteBinding {
                        hostname: hostname.to_string(),
                        peer_id,
                        mode,
                    },
                    state.clone(),
                )
                .await
            {
                Ok(ingress) => ingress,
                Err(err) => {
                    let _ = state.routes.unregister(hostname, peer_id).await;
                    return Err(RegisterFailure::IngressRegistrationFailed(err.to_string()));
                }
            };

            if mode == IngressMode::Tcp {
                if let Some(port) = ingress.public_port {
                    let _ = state.routes.set_ingress_port(hostname, peer_id, port).await;
                }
            }
            Ok(RegisteredIngress {
                mode: ingress.mode,
                public_port: ingress.public_port,
            })
        }
        RegisterResult::AlreadyTaken => Err(RegisterFailure::AlreadyTaken),
    }
}

async fn send_register_err(peer: &PeerHandle, hostname: &str, reason: impl Into<String>) {
    let reason = reason.into();
    let hostname = hostname.to_string();
    if let Err(err) = peer.register_err(hostname, reason).await {
        error!(peer_id = %peer.id, error = %err, "failed to send register err");
    }
}

struct AuthState {
    required: bool,
    authenticated: bool,
    subject: Option<String>,
    agent_label: Option<String>,
    nonce: Option<Vec<u8>>,
    issued_at: Option<Instant>,
}

impl AuthState {
    fn new(insecure: bool) -> Self {
        Self {
            required: !insecure,
            authenticated: insecure,
            subject: None,
            agent_label: None,
            nonce: None,
            issued_at: None,
        }
    }

    fn set_challenge(&mut self, nonce: Vec<u8>) {
        self.nonce = Some(nonce);
        self.issued_at = Some(Instant::now());
    }

    async fn verify_and_bind(
        &mut self,
        peer: Arc<PeerHandle>,
        state: Arc<EdgeState>,
        method: frok_protocol::AuthMethod,
    ) -> Result<String, String> {
        let nonce = self
            .nonce
            .as_ref()
            .ok_or_else(|| "auth challenge missing".to_string())?;
        let issued_at = self
            .issued_at
            .ok_or_else(|| "auth challenge missing".to_string())?;

        let result = state
            .auth
            .verify(&state.store, method, nonce, issued_at)
            .await
            .map_err(|err| err.to_string())?;

        let now = now_ts().map_err(|err| err.to_string())?;
        let existing = state
            .store
            .load_identity(&result.subject)
            .map_err(|err| err.to_string())?;
        let record = if let Some(existing) = existing {
            IdentityRecord {
                subject: existing.subject,
                auth_mode: existing.auth_mode,
                agent_label: result.agent_label.clone(),
                first_seen_at: existing.first_seen_at,
                last_seen_at: now,
                public_key_fingerprint: result.public_key_fingerprint.clone(),
            }
        } else {
            IdentityRecord {
                subject: result.subject.clone(),
                auth_mode: result.auth_kind,
                agent_label: result.agent_label.clone(),
                first_seen_at: now,
                last_seen_at: now,
                public_key_fingerprint: result.public_key_fingerprint.clone(),
            }
        };

        if let Err(err) = state.store.upsert_identity(&record) {
            error!(peer_id = %peer.id, error = %err, "failed to persist identity");
        }

        self.authenticated = true;
        self.subject = Some(result.subject.clone());
        self.agent_label = Some(result.agent_label);
        Ok(result.subject)
    }
}
