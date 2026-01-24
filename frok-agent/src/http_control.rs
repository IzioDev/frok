use crate::connection::{AgentSender, InflightMap};
use crate::http_forward::handle_http_request;
use crate::proxy::{BodyStream, HttpProxy};
use crate::route_spec::RouteSpec;
use crate::ui::AppState;
use async_stream::stream;
use bytes::Bytes;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Notify, RwLock};

use frok_protocol::HttpRequestBody;

pub(crate) struct BodyQueue {
    inner: Mutex<VecDeque<Bytes>>,
    notify: Notify,
    closed: AtomicBool,
}

impl BodyQueue {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn stream(self: Arc<Self>) -> BodyStream {
        Box::pin(stream! {
            loop {
                let next = {
                    let mut guard = self.inner.lock().await;
                    guard.pop_front()
                };

                if let Some(bytes) = next {
                    if !bytes.is_empty() {
                        yield bytes;
                    }
                    continue;
                }

                if self.closed.load(Ordering::Acquire) {
                    break;
                }

                self.notify.notified().await;
            }
        })
    }

    pub(crate) async fn push(&self, bytes: Option<Bytes>, end: bool) {
        if let Some(bytes) = bytes {
            if !bytes.is_empty() {
                let mut guard = self.inner.lock().await;
                guard.push_back(bytes);
            }
        }
        if end {
            self.closed.store(true, Ordering::Release);
        }
        self.notify.notify_one();
    }
}

pub async fn handle_request_start(
    request: frok_protocol::HttpRequestStart,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    inflight: InflightMap,
    proxy: Arc<HttpProxy>,
) {
    let request_id = request.request_id;
    let queue = BodyQueue::new();
    inflight.insert(request_id, queue.clone());

    let sender = sender.clone();
    let state = state.clone();
    let active_routes = active_routes.clone();
    let inflight = inflight.clone();
    tokio::spawn(async move {
        let body_stream: BodyStream = queue.stream();
        handle_http_request(
            request,
            body_stream,
            active_routes,
            proxy,
            sender.as_ref(),
            state,
        )
        .await;
        inflight.remove(&request_id);
    });
}

pub async fn handle_request_body(
    body: HttpRequestBody,
    inflight: InflightMap,
    state: Arc<AppState>,
) {
    let queue = inflight.get(&body.request_id).map(|entry| entry.clone());
    if let Some(queue) = queue {
        let bytes = if body.chunk.is_empty() {
            None
        } else {
            Some(Bytes::from(body.chunk))
        };
        queue.push(bytes, body.end).await;
        if body.end {
            inflight.remove(&body.request_id);
        }
    } else {
        state
            .log_warn(format!("request body for unknown id {}", body.request_id))
            .await;
    }
}
