use std::net::TcpStream;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

use super::super::app_state::{
    AddRouteState, AddRouteStep, LocalTarget, ToastKind, UiOverlay, UiSnapshot, UiState,
};
use super::super::commands::Command;
use crate::host::resolve_public_host;
use frok_protocol::IngressMode;

use super::OverlayAction;

pub(super) fn handle_add_route_key(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    _state: &Arc<super::super::AppState>,
    tx: &UnboundedSender<Command>,
    wizard: &mut AddRouteState,
) -> OverlayAction {
    if key.code == KeyCode::Esc {
        return OverlayAction::Close;
    }

    match wizard.step {
        AddRouteStep::Target => match key.code {
            KeyCode::Up => {
                wizard.target_selection = wizard.target_selection.saturating_sub(1);
            }
            KeyCode::Down => {
                if wizard.target_selection + 1 < wizard.targets.len() {
                    wizard.target_selection += 1;
                }
            }
            KeyCode::Enter => {
                if is_manual_target(wizard) {
                    if wizard.target_manual.value.trim().is_empty() {
                        ui.set_toast("Enter a target address", ToastKind::Warn);
                    } else {
                        wizard.step = AddRouteStep::Mode;
                    }
                } else {
                    wizard.step = AddRouteStep::Mode;
                }
            }
            KeyCode::Backspace => {
                if is_manual_target(wizard) {
                    wizard.target_manual.backspace();
                }
            }
            KeyCode::Delete => {
                if is_manual_target(wizard) {
                    wizard.target_manual.delete();
                }
            }
            KeyCode::Left => {
                if is_manual_target(wizard) {
                    wizard.target_manual.move_left();
                }
            }
            KeyCode::Right => {
                if is_manual_target(wizard) {
                    wizard.target_manual.move_right();
                }
            }
            KeyCode::Home => {
                if is_manual_target(wizard) {
                    wizard.target_manual.move_home();
                }
            }
            KeyCode::End => {
                if is_manual_target(wizard) {
                    wizard.target_manual.move_end();
                }
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return OverlayAction::Keep;
                }
                if is_manual_target(wizard) {
                    wizard.target_manual.insert_char(c);
                }
            }
            _ => {}
        },
        AddRouteStep::Mode => match key.code {
            KeyCode::Up => {
                wizard.mode_selection = wizard.mode_selection.saturating_sub(1);
            }
            KeyCode::Down => {
                if wizard.mode_selection < 2 {
                    wizard.mode_selection += 1;
                }
            }
            KeyCode::Left => {
                wizard.step = AddRouteStep::Target;
            }
            KeyCode::Enter => {
                wizard.step = AddRouteStep::Name;
            }
            _ => {}
        },
        AddRouteStep::Name => match key.code {
            KeyCode::Left => {
                wizard.step = AddRouteStep::Mode;
            }
            KeyCode::Enter => {
                if let Err(err) = validate_route_name(snapshot, wizard) {
                    wizard.error = Some(err);
                } else {
                    wizard.error = None;
                    wizard.step = AddRouteStep::Confirm;
                }
            }
            KeyCode::Tab => {
                if wizard.name_input.value.trim().is_empty() {
                    let suggestion = suggest_name_from_target(wizard);
                    wizard.name_input.set(suggestion.to_string());
                }
            }
            KeyCode::Backspace => {
                wizard.name_input.backspace();
            }
            KeyCode::Delete => {
                wizard.name_input.delete();
            }
            KeyCode::Right => {
                wizard.name_input.move_right();
            }
            KeyCode::Home => {
                wizard.name_input.move_home();
            }
            KeyCode::End => {
                wizard.name_input.move_end();
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    return OverlayAction::Keep;
                }
                wizard.name_input.insert_char(c);
            }
            _ => {}
        },
        AddRouteStep::Confirm => match key.code {
            KeyCode::Left => {
                wizard.step = AddRouteStep::Name;
            }
            KeyCode::Enter => {
                let Some(local_addr) = selected_target_addr(wizard) else {
                    ui.set_toast("Missing local target", ToastKind::Warn);
                    wizard.step = AddRouteStep::Target;
                    return OverlayAction::Keep;
                };
                let name = wizard.name_input.value.trim().to_string();
                let mode = mode_from_selection(wizard.mode_selection);
                let _ = tx.send(Command::Register {
                    name,
                    local_addr,
                    mode,
                });
                ui.set_toast("ROUTE ARMED.", ToastKind::Info);
                return OverlayAction::Close;
            }
            _ => {}
        },
    }

    OverlayAction::Keep
}

pub(super) fn open_add_route(ui: &mut UiState) {
    let (overlay, scan_rx) = build_add_route_overlay();
    ui.overlay = overlay;
    ui.target_scan = scan_rx;
}

pub(super) fn build_add_route_overlay() -> (UiOverlay, Option<mpsc::Receiver<Vec<LocalTarget>>>) {
    let targets = build_common_targets_stub();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let scanned = build_common_targets_scanned();
        let _ = tx.send(scanned);
    });
    (UiOverlay::AddRoute(AddRouteState::new(targets)), Some(rx))
}

fn build_common_targets_stub() -> Vec<LocalTarget> {
    let ports = [3000u16, 5173, 8080, 8000, 5000, 4000, 9222, 80, 443];
    let mut targets = Vec::new();
    for port in ports {
        let addr = format!("127.0.0.1:{port}");
        let label = format!("localhost:{port}");
        targets.push(LocalTarget {
            label,
            addr,
            open: None,
        });
    }
    targets.push(LocalTarget {
        label: "Custom...".to_string(),
        addr: String::new(),
        open: None,
    });
    targets
}

fn build_common_targets_scanned() -> Vec<LocalTarget> {
    let ports = [3000u16, 5173, 8080, 8000, 5000, 4000, 9222, 80, 443];
    let mut targets = Vec::new();
    for port in ports {
        let addr = format!("127.0.0.1:{port}");
        let label = format!("localhost:{port}");
        let open = is_port_open(port);
        targets.push(LocalTarget {
            label,
            addr,
            open: Some(open),
        });
    }
    targets.push(LocalTarget {
        label: "Custom...".to_string(),
        addr: String::new(),
        open: None,
    });
    targets
}

fn is_port_open(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(30)).is_ok()
}

fn is_manual_target(wizard: &AddRouteState) -> bool {
    wizard
        .targets
        .get(wizard.target_selection)
        .map(|target| target.addr.is_empty())
        .unwrap_or(false)
}

fn selected_target_addr(wizard: &AddRouteState) -> Option<String> {
    let target = wizard.targets.get(wizard.target_selection)?;
    if target.addr.is_empty() {
        normalize_manual_target(&wizard.target_manual.value)
    } else {
        Some(target.addr.clone())
    }
}

fn normalize_manual_target(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("127.0.0.1:{trimmed}"));
    }
    Some(trimmed.to_string())
}

fn mode_from_selection(selection: usize) -> IngressMode {
    match selection {
        1 => IngressMode::Http2,
        2 => IngressMode::Tcp,
        _ => IngressMode::Http1,
    }
}

fn suggest_name_from_target(wizard: &AddRouteState) -> &'static str {
    if let Some(addr) = selected_target_addr(wizard) {
        if let Some(port) = extract_port(&addr) {
            if port == 80 || port == 443 {
                return "web";
            }
        }
    }
    "app"
}

fn extract_port(addr: &str) -> Option<u16> {
    if let Ok(parsed) = addr.parse::<std::net::SocketAddr>() {
        return Some(parsed.port());
    }
    let port = addr.rsplit_once(':').map(|(_, port)| port)?;
    port.parse::<u16>().ok()
}

fn validate_route_name(snapshot: &UiSnapshot, wizard: &AddRouteState) -> Result<(), String> {
    let name = wizard.name_input.value.trim();
    if name.is_empty() {
        return Err("Name is required".to_string());
    }
    if snapshot.routes.iter().any(|route| route.name == name) {
        return Err("Name already in use".to_string());
    }
    resolve_public_host(name, snapshot.public_domain.as_str())
        .map(|_| ())
        .map_err(|err| err.to_string())
}
