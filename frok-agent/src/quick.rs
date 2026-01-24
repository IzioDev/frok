use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::cli::QuickCommand;
use crate::connection::{ConnectionManager, SenderRouter, connection_supervisor};
use crate::route_spec::RouteSpec;
use crate::routes::{format_public_url, request_register, resolve_quick_name};
use crate::ui::{AppState, RouteEvent, RouteView};

pub(crate) async fn run_quick_command(
    cmd: QuickCommand,
    connection_manager: Arc<ConnectionManager>,
    sender: Arc<SenderRouter>,
    state: Arc<AppState>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    public_domain: Arc<String>,
    token: CancellationToken,
) -> Result<()> {
    let mut route_rx = state.subscribe_routes();
    let resolved = resolve_quick_name(cmd.name.clone(), cmd.port, public_domain.as_str())?;

    if resolved.domain_mismatch {
        let requested = cmd
            .name
            .as_deref()
            .unwrap_or_else(|| resolved.name.as_str());
        state
            .log_warn(format!(
                "route name '{}' includes a different domain; using {}",
                requested, resolved.full_host
            ))
            .await;
    }

    let local_addr = format!("127.0.0.1:{}", cmd.port);
    let route = RouteSpec {
        local_addr,
        mode: cmd.mode,
    };

    let supervisor_handle = tokio::spawn(connection_supervisor(
        connection_manager.clone(),
        state.clone(),
        token.clone(),
    ));

    loop {
        wait_for_connection(connection_manager.clone(), token.clone()).await?;
        request_register(
            resolved.full_host.clone(),
            route.clone(),
            sender.clone(),
            state.clone(),
            pending_routes.clone(),
        )
        .await;

        let wait = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            await_registration(&mut route_rx, &resolved.name, token.clone()),
        )
        .await;

        match wait {
            Ok(Ok(view)) => {
                let url =
                    format_public_url(&view.full_host, view.ingress.mode, view.ingress.public_port);
                println!("{url}");
                if cmd.once {
                    token.cancel();
                    break;
                }
                tokio::select! {
                    _ = crate::shutdown_signal() => token.cancel(),
                    _ = token.cancelled() => {}
                }
                break;
            }
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                state.log_warn("register timed out; retrying").await;
            }
        }
    }

    let _ = supervisor_handle.await;
    Ok(())
}

async fn await_registration(
    route_rx: &mut tokio::sync::broadcast::Receiver<RouteEvent>,
    name: &str,
    token: CancellationToken,
) -> Result<RouteView> {
    loop {
        let event = tokio::select! {
            _ = token.cancelled() => {
                return Err(anyhow::anyhow!("cancelled"));
            }
            event = route_rx.recv() => event,
        };

        match event {
            Ok(RouteEvent::Registered(view)) => {
                if view.name == name {
                    return Ok(view);
                }
            }
            Ok(RouteEvent::RegisterFailed {
                name: failed,
                full_host,
                reason,
            }) => {
                if failed == name {
                    bail!("register failed for {full_host}: {reason}");
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                bail!("route event channel closed")
            }
        }
    }
}

async fn wait_for_connection(
    connection_manager: Arc<ConnectionManager>,
    token: CancellationToken,
) -> Result<()> {
    loop {
        if token.is_cancelled() {
            bail!("cancelled");
        }
        if connection_manager.is_connected().await {
            return Ok(());
        }
        let pause = tokio::time::sleep(std::time::Duration::from_millis(200));
        tokio::select! {
            _ = token.cancelled() => bail!("cancelled"),
            _ = pause => {}
        }
    }
}
