use std::collections::HashMap;
use std::io::IsTerminal;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use quinn::Endpoint;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

pub use frok_common::normalize_host;

mod auth;
mod build_info;
mod cli;
mod client_store;
mod command_loop;
mod config;
mod connection;
mod edge;
mod host;
mod http_control;
mod http_forward;
mod proxy;
mod quick;
mod request_streams;
mod route_spec;
mod routes;
mod stream_sender;
mod tcp_streams;
mod tcp_tunnel;
mod ui;

use auth::{AgentAuthConfig, AuthTracker};
use cli::{Cli, LaunchMode, RuntimeMode, select_runtime_mode};
use client_store::ClientStore;
use connection::{ConnectionManager, SenderRouter};
use proxy::HttpProxy;
use route_spec::RouteSpec;
use tcp_streams::TcpStreams;
use ui::{AppState, Command};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    let launch_mode = cli.launch_mode();
    let run_options = cli.run_options();
    let mode = match &launch_mode {
        LaunchMode::Run => select_runtime_mode(&run_options),
        LaunchMode::Quick(_) => RuntimeMode::Headless,
    };

    let logging = frok_common::logging::init_logging(frok_common::logging::LoggingConfig {
        file_prefix: match mode {
            RuntimeMode::Tui => Some("agent.log"),
            RuntimeMode::Headless => {
                if matches!(launch_mode, LaunchMode::Run) && run_options.log_file {
                    Some("agent.log")
                } else {
                    None
                }
            }
        },
        with_stdout: match mode {
            RuntimeMode::Tui => false,
            RuntimeMode::Headless => true,
        },
    })
    .context("init logging")?;

    info!(mode = ?mode, "starting frok");

    let (edge_host, edge_port) = edge::parse_edge_target(&cli.edge)?;
    let edge_addrs = edge::resolve_edge_addrs(&edge_host, edge_port).await?;
    debug!(edge = %cli.edge, edge_addrs = ?edge_addrs, "resolved edge addresses");
    let public_domain = Arc::new(
        cli.public_domain
            .clone()
            .unwrap_or_else(|| default_public_domain(&edge_host)),
    );

    let token = CancellationToken::new();
    let state = AppState::new();
    state
        .set_public_domain(public_domain.as_ref().clone())
        .await;
    state.set_edge(cli.edge.clone()).await;
    state.set_agent_label(cli.agent_label.clone()).await;
    let agent_build = build_info::local_build_info();
    state.set_agent_build(agent_build.clone()).await;
    state
        .log_warn(format!(
            "{} ({}) - {}",
            build_info::ALPHA_NOTICE,
            build_info::format_build(&agent_build),
            build_info::GITHUB_URL
        ))
        .await;

    let store = match ClientStore::load() {
        Ok(store) => store,
        Err(err) => {
            state
                .log_error(format!("route store load failed: {err}"))
                .await;
            ClientStore::empty()
        }
    };
    let store = Arc::new(Mutex::new(store));

    let client_config = edge::configure_client_transport(edge::build_client_config()?)?;

    let has_v4 = edge_addrs
        .iter()
        .any(|addr| matches!(addr, SocketAddr::V4(_)));
    let has_v6 = edge_addrs
        .iter()
        .any(|addr| matches!(addr, SocketAddr::V6(_)));

    let mut client_v4 = if has_v4 {
        let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        Some(Endpoint::client(bind_addr)?)
    } else {
        None
    };
    let mut client_v6 = if has_v6 {
        let bind_addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0);
        Some(Endpoint::client(bind_addr)?)
    } else {
        None
    };

    if let Some(client) = &mut client_v4 {
        client.set_default_client_config(client_config.clone());
    }
    if let Some(client) = &mut client_v6 {
        client.set_default_client_config(client_config);
    }

    let sender = SenderRouter::new();
    let auth_tracker = AuthTracker::new();
    let auth_config = AgentAuthConfig {
        agent_label: cli.agent_label.clone(),
        oidc_code: cli.oidc_code.clone(),
        manual_tty: mode == RuntimeMode::Headless && std::io::stdin().is_terminal(),
        manual_ui: mode == RuntimeMode::Tui,
    };

    let active_routes = Arc::new(RwLock::new(HashMap::<String, RouteSpec>::new()));
    let pending_routes = Arc::new(Mutex::new(HashMap::<String, RouteSpec>::new()));
    let inflight = Arc::new(dashmap::DashMap::new());
    let proxy = Arc::new(HttpProxy::new());
    let tcp_streams = TcpStreams::new();

    let connection_manager = Arc::new(ConnectionManager::new(
        client_v4,
        client_v6,
        edge_host,
        edge_port,
        edge_addrs,
        public_domain.clone(),
        state.clone(),
        sender.clone(),
        active_routes.clone(),
        pending_routes.clone(),
        inflight.clone(),
        proxy.clone(),
        tcp_streams.clone(),
        store.clone(),
        auth_config.clone(),
        auth_tracker.clone(),
        token.clone(),
    ));

    match launch_mode {
        LaunchMode::Run => {
            let cmd_tx = mpsc::unbounded_channel::<Command>();
            let ui_handle = match mode {
                RuntimeMode::Tui => {
                    Some(ui::start_ui(state.clone(), cmd_tx.0.clone(), token.clone()))
                }
                RuntimeMode::Headless => None,
            };

            let supervisor_handle = if mode == RuntimeMode::Headless {
                Some(tokio::spawn(connection::connection_supervisor(
                    connection_manager.clone(),
                    state.clone(),
                    token.clone(),
                )))
            } else {
                if let Err(err) = connection_manager.connect(token.clone()).await {
                    if !token.is_cancelled() {
                        state
                            .log_error(format!("connect failed: {err} (use `connect` to retry)"))
                            .await;
                    }
                }
                None
            };

            let command_handle = tokio::spawn(command_loop::command_loop(
                cmd_tx.1,
                sender.clone(),
                connection_manager.clone(),
                state.clone(),
                active_routes.clone(),
                pending_routes.clone(),
                store.clone(),
                public_domain.clone(),
                token.clone(),
            ));

            tokio::select! {
                _ = shutdown_signal() => token.cancel(),
                _ = token.cancelled() => {}
            }

            token.cancel();
            let _ = command_handle.await;
            connection_manager.shutdown().await;
            if let Some(handle) = ui_handle {
                let _ = handle.await;
            }
            if let Some(handle) = supervisor_handle {
                let _ = handle.await;
            }
        }
        LaunchMode::Quick(cmd) => {
            quick::run_quick_command(
                cmd,
                connection_manager.clone(),
                sender.clone(),
                state.clone(),
                pending_routes.clone(),
                public_domain.clone(),
                token.clone(),
            )
            .await?;
            connection_manager.shutdown().await;
        }
    }

    drop(logging);

    Ok(())
}

pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = term.recv() => {},
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to install SIGTERM handler");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn default_public_domain(edge_host: &str) -> String {
    let host = normalize_host(edge_host).into_owned();
    let mut parts = host.split('.').collect::<Vec<_>>();
    if parts.len() >= 2 && parts[0] == "edge" {
        parts.remove(0);
        let candidate = parts.join(".");
        if !candidate.is_empty() {
            return candidate;
        }
    }
    host
}
