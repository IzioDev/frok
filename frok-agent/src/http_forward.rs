use crate::proxy::{BodyStream, HttpProxy, ResponseHandle};
use crate::route_spec::RouteSpec;
use crate::ui::AppState;
use bytes::Bytes;
use http_body_util::BodyExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use frok_common::http::{extract_host, is_hop_header};
use frok_protocol::{
    ByteBuf, ClientCommands, HttpRequestStart, HttpResponseBody, HttpResponseStart, IngressMode,
    MAX_BODY_CHUNK,
};

pub async fn handle_http_request<S: ClientCommands + Sync>(
    request: HttpRequestStart,
    body_stream: BodyStream,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    proxy: Arc<HttpProxy>,
    sender: &S,
    state: Arc<AppState>,
) {
    let request_id = request.request_id;
    let host = extract_host(&request.headers);
    let host = match host {
        Some(host) => host,
        None => {
            state.log_warn("http request missing host").await;
            send_error_response(sender, request_id, 400, "missing host").await;
            return;
        }
    };

    let route = {
        let guard = active_routes.read().await;
        guard.get(host.as_ref()).cloned()
    };

    let route = match route {
        Some(route) => route,
        None => {
            state.log_warn(format!("no route for host: {host}")).await;
            send_error_response(sender, request_id, 502, "unknown host").await;
            return;
        }
    };

    if route.mode == IngressMode::Tcp {
        state
            .log_warn(format!("tcp route received http request: {host}"))
            .await;
        send_error_response(sender, request_id, 400, "route expects tcp").await;
        return;
    }

    let response = match proxy
        .forward_streaming(&route.local_addr, route.mode, request, body_stream)
        .await
    {
        Ok(response) => response,
        Err(err) => {
            state.log_error(format!("proxy error: {err}")).await;
            send_error_response(sender, request_id, 502, "bad gateway").await;
            return;
        }
    };

    stream_response(response, sender, state).await;
}

pub async fn stream_response<S: ClientCommands + Sync>(
    response: ResponseHandle,
    sender: &S,
    state: Arc<AppState>,
) {
    let status = response.response.status().as_u16();
    let mut headers = Vec::with_capacity(response.response.headers().len());
    for (name, value) in response.response.headers().iter() {
        if is_hop_header(name.as_str()) || name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        headers.push(frok_protocol::Header {
            name: ByteBuf::from(Bytes::copy_from_slice(name.as_str().as_bytes())),
            value: ByteBuf::from(Bytes::copy_from_slice(value.as_bytes())),
        });
    }

    let start = HttpResponseStart {
        request_id: response.request_id,
        status,
        headers,
    };

    if sender.http_response_start(start).await.is_err() {
        state.log_error("http response start send failed").await;
        let _ = response.done.send(());
        return;
    }

    let mut body = response.response.into_body();
    while let Some(frame) = body.frame().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(err) => {
                state
                    .log_warn(format!("http response body error: {err}"))
                    .await;
                break;
            }
        };

        let data = match frame.into_data() {
            Ok(data) => data,
            Err(_) => continue,
        };

        if send_body_chunks(sender, response.request_id, data)
            .await
            .is_err()
        {
            state.log_error("http response send failed").await;
            break;
        }
    }

    let _ = sender
        .http_response_body(HttpResponseBody {
            request_id: response.request_id,
            chunk: ByteBuf::empty(),
            end: true,
        })
        .await;

    let _ = response.done.send(());
}

async fn send_error_response<S: ClientCommands + Sync>(
    sender: &S,
    request_id: u64,
    status: u16,
    message: &'static str,
) {
    let start = HttpResponseStart {
        request_id,
        status,
        headers: Vec::new(),
    };
    let _ = sender.http_response_start(start).await;
    let _ = sender
        .http_response_body(HttpResponseBody {
            request_id,
            chunk: ByteBuf::from_static(message.as_bytes()),
            end: true,
        })
        .await;
}

async fn send_body_chunks<S: ClientCommands + Sync>(
    sender: &S,
    request_id: u64,
    data: Bytes,
) -> Result<(), S::Error> {
    let mut start = 0;
    while start < data.len() {
        let end = (start + MAX_BODY_CHUNK).min(data.len());
        let chunk = data.slice(start..end);
        sender
            .http_response_body(HttpResponseBody {
                request_id,
                chunk: ByteBuf::from(chunk),
                end: false,
            })
            .await?;
        start = end;
    }
    Ok(())
}
