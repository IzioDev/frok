use std::convert::Infallible;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{ProxyMode, ServiceConfig};

pub(crate) struct ServiceHandle {
    pub(crate) addr: SocketAddr,
    token: CancellationToken,
    task: JoinHandle<()>,
}

impl ServiceHandle {
    pub(crate) async fn shutdown(self) {
        self.token.cancel();
        let _ = self.task.await;
    }
}

pub(crate) async fn start_service(
    mode: ProxyMode,
    config: &ServiceConfig,
) -> Result<ServiceHandle> {
    let bind_addr = config
        .bind_addr
        .unwrap_or_else(|| "127.0.0.1:0".parse().expect("parse bind addr"));
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind service on {bind_addr}"))?;
    let addr = listener.local_addr().context("service local addr")?;
    let token = CancellationToken::new();
    let body_bytes = Bytes::from(vec![b'a'; config.response_bytes]);

    let task_token = token.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = task_token.cancelled() => break,
                accept = listener.accept() => {
                    let (stream, _) = match accept {
                        Ok(pair) => pair,
                        Err(_) => continue,
                    };
                    let body_bytes = body_bytes.clone();
                    let service = service_fn(move |_req| {
                        let body = Full::new(body_bytes.clone());
                        async move { Ok::<_, Infallible>(Response::new(body)) }
                    });
                    let io = TokioIo::new(stream);
                    tokio::spawn(async move {
                        let result = match mode {
                            ProxyMode::Http1 => http1::Builder::new().serve_connection(io, service).await,
                            ProxyMode::Http2 => http2::Builder::new(TokioExecutor::new())
                                .serve_connection(io, service)
                                .await,
                        };
                        let _ = result;
                    });
                }
            }
        }
    });

    Ok(ServiceHandle { addr, token, task })
}
