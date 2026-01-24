use crate::http_forward::handle_http_request;
use crate::proxy::{BodyStream, HttpProxy};
use crate::route_spec::RouteSpec;
use crate::stream_sender::StreamSender;
use crate::ui::AppState;
use async_stream::stream;
use bytes::Bytes;
use quinn::{Connection, RecvStream, SendStream};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use frok_common::wire::read_wire_message;
use frok_protocol::{ServerMessage, WireMessage};

pub async fn accept_request_streams(
    connection: Connection,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    proxy: Arc<HttpProxy>,
    token: CancellationToken,
) {
    loop {
        let incoming = tokio::select! {
            _ = token.cancelled() => break,
            incoming = connection.accept_bi() => incoming,
        };

        let (send, recv) = match incoming {
            Ok(pair) => pair,
            Err(err) => {
                state.log_error(format!("stream accept error: {err}")).await;
                break;
            }
        };

        let state = state.clone();
        let active_routes = active_routes.clone();
        let proxy = proxy.clone();
        tokio::spawn(async move {
            handle_request_stream(send, recv, active_routes, proxy, state).await;
        });
    }
}

async fn handle_request_stream(
    send: SendStream,
    mut recv: RecvStream,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    proxy: Arc<HttpProxy>,
    state: Arc<AppState>,
) {
    let mut recv_buf = Vec::new();
    let message = match read_wire_message(&mut recv, &mut recv_buf).await {
        Ok(message) => message,
        Err(err) => {
            state.log_error(format!("stream read error: {err}")).await;
            return;
        }
    };

    let request = match message {
        WireMessage::Server(ServerMessage::HttpRequestStart { request }) => request,
        _ => {
            state.log_warn("stream received unexpected message").await;
            return;
        }
    };

    let body_stream = request_body_stream(recv, recv_buf);
    let sender = StreamSender::new(send);
    handle_http_request(request, body_stream, active_routes, proxy, &sender, state).await;
    sender.finish().await;
}

fn request_body_stream(mut recv: RecvStream, mut buffer: Vec<u8>) -> BodyStream {
    Box::pin(stream! {
        loop {
            let message = match read_wire_message(&mut recv, &mut buffer).await {
                Ok(message) => message,
                Err(_) => break,
            };
            match message {
                WireMessage::Server(ServerMessage::HttpRequestBody { body }) => {
                    let bytes: Bytes = body.chunk.into();
                    if !bytes.is_empty() {
                        yield bytes;
                    }
                    if body.end {
                        break;
                    }
                }
                _ => {}
            }
        }
    })
}
