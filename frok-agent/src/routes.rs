use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use frok_common::normalize_host;

use crate::connection::SenderRouter;
use crate::host::{normalize_route_name, resolve_public_host};
use crate::route_spec::RouteSpec;
use crate::ui::{AppState, RouteEntry, RouteStatus};
use frok_protocol::{ClientCommands, IngressMode};

pub(crate) fn format_public_url(host: &str, mode: IngressMode, public_port: Option<u16>) -> String {
    match mode {
        IngressMode::Tcp => match public_port {
            Some(port) => format!("tcp://{host}:{port}"),
            None => format!("tcp://{host}"),
        },
        _ => format!("https://{host}"),
    }
}

pub(crate) fn resolve_quick_name(
    preferred: Option<String>,
    port: u16,
    public_domain: &str,
) -> Result<crate::host::ResolvedHost> {
    if let Some(name) = preferred {
        return resolve_public_host(&name, public_domain);
    }

    let mut candidates = Vec::new();
    if let Some(cwd) = current_dir_name() {
        candidates.push(cwd);
    }
    candidates.push(match port {
        80 | 443 => "web".to_string(),
        _ => "app".to_string(),
    });

    for candidate in candidates {
        if let Ok(resolved) = resolve_public_host(&candidate, public_domain) {
            return Ok(resolved);
        }
    }

    resolve_public_host("app", public_domain)
}

fn current_dir_name() -> Option<String> {
    let dir = std::env::current_dir().ok()?;
    let name = dir.file_name()?.to_string_lossy();
    sanitize_label(&name)
}

fn sanitize_label(input: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if ch == '-' || ch == '_' || ch == ' ' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        return None;
    }
    let mut label = trimmed.to_string();
    if label.len() > 63 {
        label.truncate(63);
    }
    Some(label)
}

pub(crate) async fn restore_routes(
    sender: Arc<SenderRouter>,
    state: Arc<AppState>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
    store: Arc<Mutex<crate::client_store::ClientStore>>,
    public_domain: Arc<String>,
) {
    let routes = {
        let guard = store.lock().await;
        guard.routes()
    };

    if routes.is_empty() {
        return;
    }

    state
        .log_info(format!("restoring {} stored routes", routes.len()))
        .await;

    for (name, route) in routes {
        match resolve_public_host(&name, public_domain.as_str()) {
            Ok(resolved) => {
                request_register(
                    resolved.full_host,
                    route,
                    sender.clone(),
                    state.clone(),
                    pending_routes.clone(),
                )
                .await;
            }
            Err(err) => {
                state
                    .log_error(format!("invalid stored route '{name}': {err}"))
                    .await;
            }
        }
    }
}

pub(crate) async fn request_register(
    hostname: String,
    route: RouteSpec,
    sender: Arc<SenderRouter>,
    state: Arc<AppState>,
    pending_routes: Arc<Mutex<HashMap<String, RouteSpec>>>,
) {
    let host = normalize_host(&hostname);
    let name = normalize_route_name(host.as_ref()).into_owned();
    let host_owned = host.into_owned();
    {
        let mut guard = pending_routes.lock().await;
        guard.insert(host_owned.clone(), route.clone());
    }
    if !name.is_empty() {
        let entry = RouteEntry {
            name,
            public_url: format_public_url(&host_owned, route.mode, None),
            local_addr: route.local_addr.clone(),
            mode: route.mode,
            status: RouteStatus::Pending,
        };
        state.register_route(entry).await;
    }
    state
        .log_info(format!(
            "register requested: {host_owned} -> {} ({})",
            route.local_addr,
            route.mode.label()
        ))
        .await;

    if let Err(err) = sender
        .register(host_owned, route.local_addr.clone(), route.mode)
        .await
    {
        state
            .log_error(format!("register send failed: {err}"))
            .await;
    }
}
