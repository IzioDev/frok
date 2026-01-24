use crate::connection::AgentSender;
use crate::route_spec::RouteSpec;
use crate::tcp_streams::TcpStreamEvent;
use crate::tcp_streams::TcpStreams;
use crate::ui::AppState;
use bytes::{Bytes, BytesMut};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc};

use frok_protocol::{
    ByteBuf, ClientCommands, IngressMode, MAX_BODY_CHUNK, TcpStreamData, TcpStreamOpen,
};

const TCP_TUNNEL_CHANNEL_CAPACITY: usize = 128;

pub async fn handle_tcp_open(
    stream: TcpStreamOpen,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    tcp_streams: Arc<TcpStreams>,
) {
    let host = crate::normalize_host(&stream.hostname);
    let route = {
        let guard = active_routes.read().await;
        guard.get(host.as_ref()).cloned()
    };

    let route = match route {
        Some(route) => route,
        None => {
            let _ = sender.tcp_stream_end(stream.stream_id).await;
            state
                .log_warn(format!("tcp stream for unknown host: {host}"))
                .await;
            return;
        }
    };

    if route.mode != IngressMode::Tcp {
        let _ = sender.tcp_stream_end(stream.stream_id).await;
        state
            .log_warn(format!("tcp stream for non-tcp route: {host}"))
            .await;
        return;
    }

    let local = match TcpStream::connect(&route.local_addr).await {
        Ok(stream) => stream,
        Err(err) => {
            let _ = sender.tcp_stream_end(stream.stream_id).await;
            state
                .log_error(format!("tcp connect failed ({host}): {err}"))
                .await;
            return;
        }
    };

    let (mut reader, mut writer) = local.into_split();
    let (tx, mut rx) = mpsc::channel(TCP_TUNNEL_CHANNEL_CAPACITY);
    tcp_streams.insert(stream.stream_id, tx).await;

    let sender_read = sender.clone();
    let tcp_streams_read = tcp_streams.clone();
    let stream_id = stream.stream_id;
    let read_task = tokio::spawn(async move {
        let mut buf = BytesMut::with_capacity(MAX_BODY_CHUNK);
        loop {
            buf.clear();
            let n = match reader.read_buf(&mut buf).await {
                Ok(n) => n,
                Err(_) => 0,
            };
            if n == 0 {
                let _ = sender_read.tcp_stream_end(stream_id).await;
                break;
            }
            let chunk = buf.split_to(n).freeze();
            if sender_read
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
        tcp_streams_read.remove(stream_id).await;
    });

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

    tokio::spawn(async move {
        let _ = tokio::join!(read_task, write_task);
    });
}

pub async fn handle_tcp_data(
    data: TcpStreamData,
    tcp_streams: Arc<TcpStreams>,
    state: Arc<AppState>,
) {
    let resolved = tcp_streams
        .send_data(data.stream_id, Bytes::from(data.chunk), data.end)
        .await;
    if !resolved {
        state
            .log_warn(format!("tcp data for unknown stream {}", data.stream_id))
            .await;
    }
}
