use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::UnboundedSender;

use super::super::app_state::{PaletteState, ToastKind, UiOverlay, UiSnapshot, UiState};
use super::super::clipboard::copy_to_clipboard;
use super::super::commands::Command;
use super::add_route::open_add_route;

pub(super) fn handle_dashboard_key(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
    tx: &UnboundedSender<Command>,
) -> bool {
    match key.code {
        KeyCode::Char('q') => {
            let _ = tx.send(Command::Quit);
            return true;
        }
        KeyCode::Char('/') => {
            ui.overlay = UiOverlay::CommandPalette(PaletteState::new());
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            open_add_route(ui);
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            state.show_logs_blocking();
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            if let Some(route) = snapshot.selected_route() {
                let _ = tx.send(Command::Unregister { name: route.name });
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(route) = snapshot.selected_route() {
                match copy_to_clipboard(&route.public_url) {
                    Ok(()) => ui.set_toast("LINK COPIED.", ToastKind::Info),
                    Err(err) => ui.set_toast(format!("Copy failed: {err}"), ToastKind::Error),
                }
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
        }
        KeyCode::Char('o') | KeyCode::Char('O') => {
            if let Some(route) = snapshot.selected_route() {
                if let Err(err) = webbrowser::open(&route.public_url) {
                    ui.set_toast(format!("Open failed: {err}"), ToastKind::Error);
                }
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
        }
        KeyCode::Up => {
            if ui.route_index > 0 {
                ui.route_index -= 1;
            }
        }
        KeyCode::Down => {
            if ui.route_index + 1 < snapshot.routes.len() {
                ui.route_index += 1;
            }
        }
        _ => {}
    }
    false
}
