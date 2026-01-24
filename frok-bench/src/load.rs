use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use hdrhistogram::Histogram;
use http::Request;
use http::header::HOST;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::{http1, http2};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpStream;

use crate::config::{LoadSpec, ProxyMode};

#[derive(Debug, Clone)]
pub struct LoadReport {
    pub total_requests: u64,
    pub errors: u64,
    pub elapsed: Duration,
    pub latency: Option<LatencyReport>,
}

#[derive(Debug, Clone)]
pub struct LatencyReport {
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
}

impl LoadReport {
    pub fn requests_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            self.total_requests as f64 / secs
        } else {
            0.0
        }
    }

    pub fn error_rate(&self) -> f64 {
        if self.total_requests > 0 {
            self.errors as f64 / self.total_requests as f64
        } else {
            0.0
        }
    }
}

pub(crate) async fn run_http_load(
    edge_http_addr: std::net::SocketAddr,
    route_host: &str,
    mode: ProxyMode,
    spec: LoadSpec,
) -> Result<LoadReport> {
    if spec.total_requests == 0 {
        return Ok(LoadReport {
            total_requests: 0,
            errors: 0,
            elapsed: Duration::from_secs(0),
            latency: None,
        });
    }

    let total = spec.total_requests;
    let workers = spec.concurrency.max(1);
    let per_worker = total / workers as u64;
    let remainder = total % workers as u64;

    let start = Instant::now();

    let mut handles = Vec::with_capacity(workers);
    for i in 0..workers {
        let count = per_worker + if (i as u64) < remainder { 1 } else { 0 };
        if count == 0 {
            continue;
        }
        let route_host = route_host.to_string();
        let spec = spec.clone();
        let handle = tokio::spawn(async move {
            run_worker(edge_http_addr, &route_host, mode, spec, count)
                .await
                .unwrap_or_else(|_| WorkerReport {
                    errors: count,
                    latency: None,
                })
        });
        handles.push(handle);
    }

    let mut total_errors = 0u64;
    let mut merged_latency: Option<Histogram<u64>> = None;
    for handle in handles {
        if let Ok(report) = handle.await {
            total_errors += report.errors;
            if let Some(histogram) = report.latency {
                match merged_latency.as_mut() {
                    Some(existing) => {
                        let _ = existing.add(&histogram);
                    }
                    None => {
                        merged_latency = Some(histogram);
                    }
                }
            }
        }
    }

    let elapsed = start.elapsed();
    Ok(LoadReport {
        total_requests: total,
        errors: total_errors,
        elapsed,
        latency: merged_latency.map(LatencyReport::from_histogram),
    })
}

#[derive(Debug)]
struct WorkerReport {
    errors: u64,
    latency: Option<Histogram<u64>>,
}

async fn run_worker(
    edge_http_addr: std::net::SocketAddr,
    route_host: &str,
    mode: ProxyMode,
    spec: LoadSpec,
    requests: u64,
) -> Result<WorkerReport> {
    let stream = TcpStream::connect(edge_http_addr)
        .await
        .context("connect edge http")?;
    let io = TokioIo::new(stream);

    let mut errors = 0u64;
    let mut latency = if spec.collect_latency {
        Some(new_latency_histogram())
    } else {
        None
    };
    match mode {
        ProxyMode::Http1 => {
            let (mut sender, connection) = http1::handshake::<_, Full<Bytes>>(io)
                .await
                .context("http1 handshake")?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            let body_bytes = Bytes::from(spec.request.body.clone());
            for _ in 0..requests {
                let req = build_request_http1(route_host, &spec, body_bytes.clone())?;
                let started = Instant::now();
                let response = match sender.send_request(req).await {
                    Ok(response) => response,
                    Err(_) => {
                        errors += 1;
                        continue;
                    }
                };
                let status = response.status();
                let _ = response.into_body().collect().await;
                record_latency(&mut latency, started.elapsed());
                if !status.is_success() {
                    errors += 1;
                }
            }
        }
        ProxyMode::Http2 => {
            let (mut sender, connection) =
                http2::handshake::<_, _, Full<Bytes>>(TokioExecutor::new(), io)
                    .await
                    .context("http2 handshake")?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            let body_bytes = Bytes::from(spec.request.body.clone());
            for _ in 0..requests {
                let req = build_request_http2(route_host, &spec, body_bytes.clone())?;
                let started = Instant::now();
                let response = match sender.send_request(req).await {
                    Ok(response) => response,
                    Err(_) => {
                        errors += 1;
                        continue;
                    }
                };
                let status = response.status();
                let _ = response.into_body().collect().await;
                record_latency(&mut latency, started.elapsed());
                if !status.is_success() {
                    errors += 1;
                }
            }
        }
    }

    Ok(WorkerReport { errors, latency })
}

fn new_latency_histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, 600_000_000, 3).expect("latency histogram")
}

fn record_latency(histogram: &mut Option<Histogram<u64>>, duration: Duration) {
    let Some(histogram) = histogram.as_mut() else {
        return;
    };
    let micros = duration.as_micros().min(u64::MAX as u128) as u64;
    let value = micros.max(1);
    let capped = value.min(histogram.high());
    let _ = histogram.record(capped);
}

impl LatencyReport {
    fn from_histogram(histogram: Histogram<u64>) -> Self {
        Self {
            p50: micros_to_duration(histogram.value_at_quantile(0.50)),
            p95: micros_to_duration(histogram.value_at_quantile(0.95)),
            p99: micros_to_duration(histogram.value_at_quantile(0.99)),
            min: micros_to_duration(histogram.min()),
            max: micros_to_duration(histogram.max()),
            mean: micros_to_duration(histogram.mean().round() as u64),
        }
    }
}

fn micros_to_duration(micros: u64) -> Duration {
    Duration::from_micros(micros)
}

fn build_request_http1(host: &str, spec: &LoadSpec, body: Bytes) -> Result<Request<Full<Bytes>>> {
    let mut builder = Request::builder().method(spec.request.method.clone());
    builder = builder.uri(spec.request.path.clone());
    let req = builder
        .header(HOST, host)
        .body(Full::new(body))
        .context("build http1 request")?;
    Ok(req)
}

fn build_request_http2(host: &str, spec: &LoadSpec, body: Bytes) -> Result<Request<Full<Bytes>>> {
    let uri = http::Uri::builder()
        .scheme("http")
        .authority(host)
        .path_and_query(spec.request.path.clone())
        .build()
        .context("build http2 uri")?;
    let req = Request::builder()
        .method(spec.request.method.clone())
        .uri(uri)
        .header(HOST, host)
        .body(Full::new(body))
        .context("build http2 request")?;
    Ok(req)
}
