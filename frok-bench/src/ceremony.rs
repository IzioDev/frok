use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::config::{AgentConfig, CeremonyConfig, EdgeConfig, LoadSpec, ProxyMode};
use crate::load::{LoadReport, run_http_load};
use crate::process::{ProcessHandle, spawn_agent, spawn_process};
use crate::service::{ServiceHandle, start_service};
use crate::util::{
    bin_path, ensure_dir, new_run_dir, pick_free_tcp_addr, pick_free_udp_addr, workspace_root,
};
use bytes::Bytes;
use http::Request;
use http::header::HOST;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;

#[derive(Clone, Debug)]
pub struct RouteEndpoint {
    pub name: String,
    pub mode: ProxyMode,
    pub public_host: String,
    pub local_addr: SocketAddr,
}

#[derive(Clone)]
pub struct Ceremony {
    inner: Arc<CeremonyInner>,
}

struct CeremonyInner {
    edge_http_addr: SocketAddr,
    edge_quic_addr: SocketAddr,
    routes: Vec<RouteEndpoint>,
    processes: Mutex<ProcessState>,
    services: Mutex<Vec<ServiceHandle>>,
    run_dir: PathBuf,
}

struct ProcessState {
    edge: Option<ProcessHandle>,
    agents: Vec<ProcessHandle>,
}

impl Ceremony {
    pub async fn start(config: CeremonyConfig) -> Result<Self> {
        let root = workspace_root();
        let run_dir = config.run_dir.unwrap_or_else(|| new_run_dir(&root));
        ensure_dir(&run_dir).context("create run dir")?;

        let edge_quic_addr = config
            .edge
            .quic_addr
            .unwrap_or(pick_free_udp_addr().context("pick edge quic addr")?);
        let edge_http_addr = config
            .edge
            .http_addr
            .unwrap_or(pick_free_tcp_addr().context("pick edge http addr")?);
        let edge_host = config.edge.host.clone();

        let edge_bin = resolve_edge_bin(&root, &config.edge).await?;
        let agent_bin = resolve_agent_bin(&root, &config.agents).await?;

        let edge_home = run_dir.join("edge");
        ensure_dir(&edge_home).context("create edge home")?;

        let log_dir = run_dir.join("logs");
        ensure_dir(&log_dir).context("create run logs dir")?;

        let mut edge = Some(spawn_edge(
            &edge_bin,
            &root,
            &config.edge,
            edge_quic_addr,
            edge_http_addr,
            &edge_home,
            &log_dir,
        )?);

        if let Err(err) = wait_for_tcp(edge_http_addr, config.startup_timeout, edge.as_mut())
            .await
            .context("wait for edge http")
        {
            cleanup_start(edge, Vec::new(), Vec::new()).await;
            return Err(err);
        }

        let mut agents = Vec::new();
        let mut services = Vec::new();
        let mut routes = Vec::new();

        for (idx, agent) in config.agents.iter().enumerate() {
            let service = match start_service(agent.route.mode, &agent.route.service).await {
                Ok(service) => service,
                Err(err) => {
                    cleanup_start(edge, agents, services).await;
                    return Err(err);
                }
            };
            let local_addr = service.addr;
            services.push(service);

            let agent_home = run_dir.join(format!("agent-{}", idx + 1));
            ensure_dir(&agent_home).context("create agent home")?;

            let agent_name = format!("agent-{}", idx + 1);
            let (agent_proc, _url_rx) = match spawn_agent(
                build_agent_command(
                    &agent_bin,
                    &root,
                    agent,
                    edge_quic_addr,
                    &edge_host,
                    local_addr,
                    &agent_home,
                    config.edge.insecure,
                ),
                &agent_name,
                Some(&log_dir),
            ) {
                Ok(pair) => pair,
                Err(err) => {
                    cleanup_start(edge, agents, services).await;
                    return Err(err);
                }
            };

            let public_host = match agent.route.mode {
                ProxyMode::Http1 | ProxyMode::Http2 => {
                    let host = build_public_host(&agent.route.name, &edge_host);
                    if let Err(err) = wait_for_http_route(
                        edge_http_addr,
                        &host,
                        agent.route.mode,
                        config.startup_timeout,
                    )
                    .await
                    {
                        cleanup_start(edge, agents, services).await;
                        return Err(err);
                    }
                    host
                }
            };

            routes.push(RouteEndpoint {
                name: agent.route.name.clone(),
                mode: agent.route.mode,
                public_host,
                local_addr,
            });
            agents.push(agent_proc);
        }

        Ok(Self {
            inner: Arc::new(CeremonyInner {
                edge_http_addr,
                edge_quic_addr,
                routes,
                processes: Mutex::new(ProcessState { edge, agents }),
                services: Mutex::new(services),
                run_dir,
            }),
        })
    }

    pub fn routes(&self) -> &[RouteEndpoint] {
        &self.inner.routes
    }

    pub fn edge_http_addr(&self) -> SocketAddr {
        self.inner.edge_http_addr
    }

    pub fn edge_quic_addr(&self) -> SocketAddr {
        self.inner.edge_quic_addr
    }

    pub fn run_dir(&self) -> &PathBuf {
        &self.inner.run_dir
    }

    pub async fn load_http(&self, route_name: &str, spec: LoadSpec) -> Result<LoadReport> {
        let route = self
            .inner
            .routes
            .iter()
            .find(|route| route.name == route_name)
            .ok_or_else(|| anyhow::anyhow!("unknown route {route_name}"))?;

        run_http_load(
            self.inner.edge_http_addr,
            &route.public_host,
            route.mode,
            spec,
        )
        .await
    }

    pub async fn shutdown(&self) -> Result<()> {
        let mut processes = self.inner.processes.lock().await;
        for mut agent in processes.agents.drain(..) {
            let _ = agent.kill().await;
        }
        if let Some(mut edge) = processes.edge.take() {
            let _ = edge.kill().await;
        }
        drop(processes);

        let mut services = self.inner.services.lock().await;
        for service in services.drain(..) {
            service.shutdown().await;
        }
        Ok(())
    }
}

async fn resolve_edge_bin(root: &PathBuf, edge: &EdgeConfig) -> Result<PathBuf> {
    if let Some(path) = &edge.bin_path {
        return Ok(path.clone());
    }
    let profile = bench_profile();
    build_binaries(root, &profile).await?;
    let preferred_path = bin_path(root, "frok-edge", &profile);
    if preferred_path.exists() {
        return Ok(preferred_path);
    }
    bail!("unable to locate frok-edge binary for profile {profile}")
}

async fn resolve_agent_bin(root: &PathBuf, agents: &[AgentConfig]) -> Result<PathBuf> {
    if let Some(path) = agents.iter().find_map(|agent| agent.bin_path.clone()) {
        return Ok(path);
    }
    let profile = bench_profile();
    build_binaries(root, &profile).await?;
    let preferred_path = bin_path(root, "frok", &profile);
    if preferred_path.exists() {
        return Ok(preferred_path);
    }
    bail!("unable to locate frok agent binary for profile {profile}")
}

async fn build_binaries(root: &PathBuf, profile: &str) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg("frok-edge")
        .arg("-p")
        .arg("frok-agent");
    if profile == "release" {
        cmd.arg("--release");
    }
    let status = cmd
        .current_dir(root)
        .status()
        .await
        .context("build frok binaries")?;
    if !status.success() {
        bail!("cargo build failed")
    }
    Ok(())
}

fn bench_profile() -> String {
    match std::env::var("FROK_BENCH_PROFILE").as_deref() {
        Ok("release") | Ok("RELEASE") => "release".to_string(),
        Ok("debug") | Ok("DEBUG") => "debug".to_string(),
        _ => "release".to_string(),
    }
}

fn spawn_edge(
    edge_bin: &PathBuf,
    root: &PathBuf,
    config: &EdgeConfig,
    quic_addr: SocketAddr,
    http_addr: SocketAddr,
    home_dir: &PathBuf,
    log_dir: &PathBuf,
) -> Result<ProcessHandle> {
    let (tls_cert, tls_key) = ensure_dev_tls(root)?;
    let mut cmd = Command::new(edge_bin);
    cmd.current_dir(root)
        .arg("--quic-addr")
        .arg(quic_addr.to_string())
        .arg("--http-addr")
        .arg(http_addr.to_string())
        .arg("--tls-cert")
        .arg(tls_cert)
        .arg("--tls-key")
        .arg(tls_key)
        .env("FROK_HOME", home_dir);

    if config.insecure {
        cmd.arg("--insecure");
    }

    for arg in &config.extra_args {
        cmd.arg(arg);
    }

    for (key, value) in &config.env {
        cmd.env(key, value);
    }

    if std::env::var("RUST_LOG").is_err() {
        cmd.env("RUST_LOG", "info");
    }

    spawn_process(cmd, "edge", Some(log_dir))
}

fn build_agent_command(
    agent_bin: &PathBuf,
    root: &PathBuf,
    agent: &AgentConfig,
    quic_addr: SocketAddr,
    edge_host: &str,
    local_addr: SocketAddr,
    home_dir: &PathBuf,
    insecure: bool,
) -> Command {
    let mut cmd = Command::new(agent_bin);
    cmd.current_dir(root)
        .arg("http")
        .arg(local_addr.port().to_string())
        .arg("--name")
        .arg(&agent.route.name)
        .arg("--edge")
        .arg(format!("{}:{}", edge_host, quic_addr.port()))
        .arg("--agent-label")
        .arg(&agent.label)
        .env("FROK_HOME", home_dir)
        .env("FROK_DEV_TLS", "1");

    if insecure && std::env::var("FROK_INSECURE_SKIP_AUTH").is_err() {
        cmd.env("FROK_INSECURE_SKIP_AUTH", "1");
    }

    if agent.route.mode == ProxyMode::Http2 {
        cmd.arg("--mode").arg("http2");
    }

    for arg in &agent.extra_args {
        cmd.arg(arg);
    }

    for (key, value) in &agent.env {
        cmd.env(key, value);
    }

    if std::env::var("RUST_LOG").is_err() {
        cmd.env("RUST_LOG", "info");
    }

    cmd
}

async fn wait_for_tcp(
    addr: SocketAddr,
    timeout_after: Duration,
    edge: Option<&mut ProcessHandle>,
) -> Result<()> {
    let mut edge = edge;
    let start = std::time::Instant::now();
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        if let Some(edge) = edge.as_mut() {
            if let Ok(Some(status)) = edge.try_wait() {
                let (stdout_log, stderr_log) = edge.log_paths();
                bail!(
                    "edge exited early ({status}). stdout: {} stderr: {}",
                    stdout_log
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<none>".to_string()),
                    stderr_log
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<none>".to_string())
                );
            }
        }
        if start.elapsed() > timeout_after {
            if let Some(edge) = edge.as_ref() {
                let (stdout_log, stderr_log) = edge.log_paths();
                bail!(
                    "timeout waiting for {addr}. stdout: {} stderr: {}",
                    stdout_log
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<none>".to_string()),
                    stderr_log
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<none>".to_string())
                );
            }
            bail!("timeout waiting for {addr}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn ensure_dev_tls(root: &PathBuf) -> Result<(PathBuf, PathBuf)> {
    dev_tls::load_or_generate().context("generate dev tls")?;
    let dir = root.join("target").join("quinn-dev-certs");
    let cert = dir.join("server.der");
    let key = dir.join("server-key.pkcs8.der");
    if !cert.exists() || !key.exists() {
        bail!("dev tls assets missing at {}", dir.display());
    }
    Ok((cert, key))
}

fn build_public_host(route_name: &str, edge_host: &str) -> String {
    let host = strip_port(edge_host);
    format!("{}.{}", route_name, host)
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            return &host[1..end];
        }
    }
    match host.split_once(':') {
        Some((left, _)) => left,
        None => host,
    }
}

async fn wait_for_http_route(
    edge_http_addr: SocketAddr,
    host: &str,
    _mode: ProxyMode,
    timeout_after: Duration,
) -> Result<()> {
    let start = std::time::Instant::now();
    let mut last_err = None;
    loop {
        match try_http1(edge_http_addr, host).await {
            Ok(Some(status)) => {
                if status != http::StatusCode::NOT_FOUND {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(err) => {
                last_err = Some(err);
            }
        }
        if start.elapsed() > timeout_after {
            if let Some(err) = last_err {
                bail!("timeout waiting for route {host}: {err}");
            }
            bail!("timeout waiting for route {host}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn try_http1(edge_http_addr: SocketAddr, host: &str) -> Result<Option<http::StatusCode>> {
    let stream = TcpStream::connect(edge_http_addr).await?;
    let io = TokioIo::new(stream);
    let (mut sender, connection) = http1::handshake::<_, Full<Bytes>>(io).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let req = Request::builder()
        .method("GET")
        .uri("/")
        .header(HOST, host)
        .body(Full::new(Bytes::new()))?;
    let response = sender.send_request(req).await?;
    let status = response.status();
    let _ = response.into_body().collect().await;
    Ok(Some(status))
}

async fn cleanup_start(
    mut edge: Option<ProcessHandle>,
    mut agents: Vec<ProcessHandle>,
    mut services: Vec<ServiceHandle>,
) {
    for mut agent in agents.drain(..) {
        let _ = agent.kill().await;
    }
    if let Some(mut edge) = edge.take() {
        let _ = edge.kill().await;
    }
    for service in services.drain(..) {
        service.shutdown().await;
    }
}
