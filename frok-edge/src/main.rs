use anyhow::{Context, Result, bail};
use clap::Parser;
use quinn::EndpointConfig;
use quinn::default_runtime;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::fs;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::auth::{AuthMode, EdgeAuth, KeyConfig, build_oidc_verifier};
use crate::ingress::{
    AdmissionController, AdmissionPolicy, HttpIngressManager, IngressRegistry, TcpIngressManager,
};
use crate::listener::QuicListener;
use crate::registry::PeerRegistry;
use crate::routes::RouteRegistry;
use crate::state::EdgeState;
use crate::store::EdgeStore;
use openidconnect::core::CoreJwsSigningAlgorithm;
use openidconnect::{ClientId, IssuerUrl};

mod auth;
mod build_info;
mod ingress;
mod listener;
mod peer;
mod registry;
mod routes;
mod state;
mod store;
mod tcp_streams;
mod transport;

const MAX_HTTP_CONNECTIONS: usize = 1024;
const MAX_HTTP_INFLIGHT_REQUESTS: usize = 2048;
const MAX_TCP_CONNECTIONS: usize = 1024;
const MAX_TCP_INFLIGHT_STREAMS: usize = 2048;

#[derive(Parser, Debug)]
#[command(name = "frok-edge", version, about = "Frok edge server")]
struct Cli {
    /// QUIC listen address (host:port)
    #[arg(long, env = "FROK_EDGE_QUIC_ADDR", default_value = "127.0.0.1:5000")]
    quic_addr: SocketAddr,

    /// HTTP ingress listen address (host:port)
    #[arg(long, env = "FROK_EDGE_HTTP_ADDR", default_value = "127.0.0.1:8080")]
    http_addr: SocketAddr,

    /// TLS certificate chain (PEM or DER)
    #[arg(long, env = "FROK_EDGE_TLS_CERT")]
    tls_cert: Option<PathBuf>,

    /// TLS private key (PEM or DER; DER must be PKCS#8)
    #[arg(long, env = "FROK_EDGE_TLS_KEY")]
    tls_key: Option<PathBuf>,

    /// Disable authentication (dev only)
    #[arg(long, env = "FROK_EDGE_INSECURE", default_value_t = false)]
    insecure: bool,

    /// Require OIDC authentication
    #[arg(long, env = "FROK_EDGE_OIDC_REQUIRED", default_value_t = false)]
    oidc_required: bool,

    /// Require key-based authentication
    #[arg(long, env = "FROK_EDGE_KEY_AUTH_REQUIRED", default_value_t = false)]
    key_auth_required: bool,

    /// OIDC issuer URL
    #[arg(long, env = "FROK_EDGE_OIDC_ISSUER")]
    oidc_issuer: Option<String>,

    /// OIDC audience
    #[arg(long, env = "FROK_EDGE_OIDC_AUDIENCE")]
    oidc_audience: Option<String>,

    /// Allowed JWT algorithms (comma-separated)
    #[arg(
        long,
        env = "FROK_EDGE_OIDC_ALLOWED_ALGS",
        default_value = "RS256,ES256"
    )]
    oidc_allowed_algs: String,

    /// JWKS cache TTL (seconds)
    #[arg(long, env = "FROK_EDGE_OIDC_JWKS_TTL", default_value_t = 300)]
    oidc_jwks_cache_ttl: u64,

    /// Trust-on-first-use for key auth
    #[arg(long, env = "FROK_EDGE_KEY_TOFU", default_value_t = true)]
    key_tofu: bool,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    dotenvy::dotenv().ok();
    let _logging = frok_common::logging::init_logging(frok_common::logging::LoggingConfig {
        file_prefix: Some("edge.log"),
        with_stdout: true,
    })
    .context("init logging")?;
    let cli = Cli::parse();
    let cancellation_token = CancellationToken::new();

    let edge_build = build_info::local_build_info();
    info!(
        quic_addr = %cli.quic_addr,
        http_addr = %cli.http_addr,
        version = %build_info::format_build(&edge_build),
        "starting frok-edge"
    );

    let server_config = load_server_config(&cli)?;
    let quic_addr = cli.quic_addr;
    let http_addr = cli.http_addr;

    let mut server_config = server_config;
    server_config.transport_config(Arc::new(build_transport_config()?));
    let socket = std::net::UdpSocket::bind(quic_addr)?;
    socket.set_nonblocking(true)?;
    let runtime = default_runtime().ok_or_else(|| anyhow::anyhow!("no async runtime found"))?;
    let mut endpoint_config = EndpointConfig::default();
    endpoint_config
        .max_udp_payload_size(1200)
        .context("set max udp payload size")?;
    let endpoint = quinn::Endpoint::new(endpoint_config, Some(server_config), socket, runtime)?;

    let store = EdgeStore::open().context("open edge store")?;
    let auth_mode = build_auth_mode(&cli).await.context("configure auth")?;
    let auth = EdgeAuth::new(auth_mode);
    match auth.mode() {
        AuthMode::Insecure => info!("auth mode: insecure"),
        AuthMode::Oidc(_) => info!("auth mode: oidc"),
        AuthMode::Key(config) => {
            info!(tofu = config.tofu, "auth mode: key");
        }
    }
    let peers = PeerRegistry::new();
    let routes = RouteRegistry::new();
    let tcp_streams = tcp_streams::TcpStreams::new();
    let http_admission = AdmissionController::new(AdmissionPolicy {
        max_connections: MAX_HTTP_CONNECTIONS,
        max_inflight_units: MAX_HTTP_INFLIGHT_REQUESTS,
    });
    let tcp_admission = AdmissionController::new(AdmissionPolicy {
        max_connections: MAX_TCP_CONNECTIONS,
        max_inflight_units: MAX_TCP_INFLIGHT_STREAMS,
    });
    let http_ingress =
        HttpIngressManager::new(http_addr, http_admission, cancellation_token.child_token());
    let tcp_ingress = TcpIngressManager::new(
        http_addr.ip(),
        tcp_admission,
        cancellation_token.child_token(),
    );
    let ingress = IngressRegistry::new(http_ingress, tcp_ingress);
    let state = EdgeState::new(peers, routes, tcp_streams, ingress, store, auth);
    state
        .ingress
        .start_all(state.clone())
        .await
        .context("start ingress runtimes")?;

    let quic_listener = QuicListener::new(endpoint, cancellation_token.clone(), state.clone());
    let listen_task = quic_listener.start_listen_task();

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            error!(error = %err, "unable to listen for shutdown signal");
        }
    }

    cancellation_token.cancel();
    if let Err(err) = listen_task.await {
        error!(error = %err, "listen task failed");
    }
    if let Err(err) = state.ingress.shutdown_all().await {
        error!(error = %err, "ingress shutdown failed");
    }

    info!("frok-edge shutdown complete");
    Ok(())
}

fn load_server_config(cli: &Cli) -> Result<quinn::ServerConfig> {
    match (&cli.tls_cert, &cli.tls_key) {
        (Some(cert), Some(key)) => load_cert_and_key(cert, key),
        (None, None) => {
            if cfg!(debug_assertions) {
                Ok(dev_tls::load_or_generate()?.server)
            } else {
                bail!(
                    "TLS cert/key are required in release builds. Set FROK_EDGE_TLS_CERT and FROK_EDGE_TLS_KEY or pass --tls-cert/--tls-key."
                )
            }
        }
        _ => bail!("Both --tls-cert and --tls-key must be provided when configuring TLS"),
    }
}

fn load_cert_and_key(cert_path: &Path, key_path: &Path) -> Result<quinn::ServerConfig> {
    let cert_chain = load_cert_chain(cert_path)?;
    let key = load_private_key(key_path)?;
    let server =
        quinn::ServerConfig::with_single_cert(cert_chain, key).context("server tls config")?;
    Ok(server)
}

fn load_cert_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let data = fs::read(path).with_context(|| format!("read certs from {}", path.display()))?;
    if is_pem(&data) {
        let mut reader = Cursor::new(data);
        let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .context("parse PEM certs")?;
        if certs.is_empty() {
            bail!("No certificates found in {}", path.display());
        }
        Ok(certs)
    } else {
        Ok(vec![CertificateDer::from(data)])
    }
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let data = fs::read(path).with_context(|| format!("read key from {}", path.display()))?;
    if is_pem(&data) {
        let mut reader = Cursor::new(data);
        let key = match rustls_pemfile::private_key(&mut reader).context("parse PEM private key")? {
            Some(key) => key,
            None => bail!("No private key found in {}", path.display()),
        };
        Ok(key)
    } else {
        Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(data)))
    }
}

fn is_pem(bytes: &[u8]) -> bool {
    bytes.starts_with(b"-----BEGIN")
}

fn build_transport_config() -> Result<quinn::TransportConfig, anyhow::Error> {
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(10)));
    transport.max_idle_timeout(Some(Duration::from_secs(60).try_into()?));
    transport.initial_rtt(Duration::from_millis(10));
    transport.initial_mtu(1200);
    transport.min_mtu(1200);
    transport.mtu_discovery_config(None);
    transport.send_fairness(false);
    transport.stream_receive_window(quinn::VarInt::from_u32(16 * 1024 * 1024));
    transport.receive_window(quinn::VarInt::from_u32(64 * 1024 * 1024));
    transport.send_window(64 * 1024 * 1024);
    Ok(transport)
}

async fn build_auth_mode(cli: &Cli) -> Result<AuthMode> {
    if cli.insecure {
        return Ok(AuthMode::Insecure);
    }

    if cli.oidc_required == cli.key_auth_required {
        bail!(
            "Exactly one of --oidc-required or --key-auth-required must be set (or use --insecure)"
        );
    }

    if cli.oidc_required {
        let issuer = cli
            .oidc_issuer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--oidc-issuer is required"))?;
        let audience = cli
            .oidc_audience
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--oidc-audience is required"))?;
        let ttl = Duration::from_secs(cli.oidc_jwks_cache_ttl);
        let (allowed_algs, allowed_names) = parse_allowed_algs(&cli.oidc_allowed_algs);
        let config = build_oidc_verifier(
            IssuerUrl::new(issuer)?,
            ClientId::new(audience),
            allowed_algs,
            allowed_names,
            ttl,
        )
        .await?;
        return Ok(AuthMode::Oidc(config));
    }

    Ok(AuthMode::Key(KeyConfig { tofu: cli.key_tofu }))
}

fn parse_allowed_algs(input: &str) -> (Vec<CoreJwsSigningAlgorithm>, Vec<String>) {
    let mut algs = Vec::new();
    let mut names = Vec::new();
    for entry in input.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let upper = trimmed.to_ascii_uppercase();
        let alg = match upper.as_str() {
            "RS256" => Some(CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256),
            "RS384" => Some(CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384),
            "RS512" => Some(CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512),
            "ES256" => Some(CoreJwsSigningAlgorithm::EcdsaP256Sha256),
            "ES384" => Some(CoreJwsSigningAlgorithm::EcdsaP384Sha384),
            "HS256" => Some(CoreJwsSigningAlgorithm::HmacSha256),
            "HS384" => Some(CoreJwsSigningAlgorithm::HmacSha384),
            "HS512" => Some(CoreJwsSigningAlgorithm::HmacSha512),
            "EDDSA" => Some(CoreJwsSigningAlgorithm::EdDsa),
            _ => None,
        };
        if let Some(alg) = alg {
            algs.push(alg);
            names.push(upper);
        }
    }
    (algs, names)
}
