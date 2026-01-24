use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use frok_common::normalize_host;

use crate::build_info;
use crate::client_store::ClientStore;
use crate::connection::{ConnectionManager, SenderRouter};
use crate::host::resolve_public_host;
use crate::route_spec::RouteSpec;
use crate::ui::{AppState, Command, command_specs};
use frok_protocol::ClientCommands;

pub(crate) async fn command_loop(
    mut rx: mpsc::UnboundedReceiver<Command>,
    sender: Arc<SenderRouter>,
    connection_manager: Arc<ConnectionManager>,
    state: Arc<AppState>,
    active_routes: Arc<RwLock<HashMap<String, RouteSpec>>>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    store: Arc<Mutex<ClientStore>>,
    public_domain: Arc<String>,
    token: CancellationToken,
) {
    loop {
        let cmd = tokio::select! {
            _ = token.cancelled() => break,
            cmd = rx.recv() => cmd,
        };
        let Some(cmd) = cmd else { break };
        match cmd {
            Command::Register {
                name,
                local_addr,
                mode,
            } => match resolve_public_host(&name, public_domain.as_str()) {
                Ok(resolved) => {
                    if resolved.domain_mismatch {
                        state
                            .log_warn(format!(
                                "route name '{name}' includes a different domain; using {}",
                                resolved.full_host
                            ))
                            .await;
                    }
                    crate::routes::request_register(
                        resolved.full_host,
                        RouteSpec { local_addr, mode },
                        sender.clone(),
                        state.clone(),
                        pending_routes.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    state
                        .log_error(format!("invalid route name '{name}': {err}"))
                        .await;
                }
            },
            Command::Unregister { name } => {
                let resolved = match resolve_public_host(&name, public_domain.as_str()) {
                    Ok(resolved) => resolved,
                    Err(err) => {
                        state
                            .log_error(format!("invalid route name '{name}': {err}"))
                            .await;
                        continue;
                    }
                };
                let host = normalize_host(&resolved.full_host);
                {
                    let mut guard = pending_routes.lock().await;
                    guard.remove(host.as_ref());
                }
                {
                    let mut guard = active_routes.write().await;
                    guard.remove(host.as_ref());
                }
                {
                    state.unregister_route(&resolved.name).await;
                    state
                        .log_info(format!("unregister requested: {}", resolved.full_host))
                        .await;
                }

                {
                    let result = {
                        let mut guard = store.lock().await;
                        guard.remove(&resolved.name)
                    };
                    if let Err(err) = result {
                        state
                            .log_error(format!("route store update failed: {err}"))
                            .await;
                    }
                }

                if let Err(err) = sender.unregister(host.into_owned()).await {
                    state
                        .log_error(format!("unregister send failed: {err}"))
                        .await;
                }
            }
            Command::ShowLogs => {
                state.show_logs().await;
            }
            Command::HideLogs => {
                state.hide_logs().await;
            }
            Command::ToggleLogs => {
                state.toggle_logs().await;
            }
            Command::ClearLogs => {
                state.clear_logs().await;
            }
            Command::Connect => {
                if let Err(err) = connection_manager.connect(token.clone()).await {
                    if !token.is_cancelled() {
                        state.log_error(format!("connect failed: {err}")).await;
                    }
                }
            }
            Command::OpenGithub => {
                if let Err(err) = webbrowser::open(build_info::GITHUB_URL) {
                    state.log_error(format!("github open failed: {err}")).await;
                } else {
                    state.log_info("opened GitHub repo").await;
                }
            }
            Command::Help => {
                state.log_info("commands:").await;
                for spec in command_specs() {
                    state
                        .log_info(format!("  {} - {}", spec.usage, spec.desc))
                        .await;
                }
            }
            Command::Quit => {
                token.cancel();
                break;
            }
        }
    }
}
