use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};

use super::super::app_state::{
    FirstRunState, PaletteState, ToastKind, UiOverlay, UiSnapshot, UiState,
};
use super::add_route::build_add_route_overlay;

use super::OverlayAction;

pub(super) fn handle_first_run_key(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
    first_run: &mut FirstRunState,
) -> OverlayAction {
    if snapshot.auth_url.is_some() {
        match key.code {
            KeyCode::Enter => {
                let input = first_run.auth_input.value.trim().to_string();
                if !input.is_empty() {
                    if state.send_auth_code_blocking(input) {
                        first_run.auth_input.clear();
                        ui.set_toast("AUTH CODE SENT.", ToastKind::Info);
                    } else {
                        ui.set_toast("AUTH INPUT NOT READY.", ToastKind::Warn);
                    }
                }
                return OverlayAction::Keep;
            }
            KeyCode::Backspace => {
                first_run.auth_input.backspace();
                return OverlayAction::Keep;
            }
            KeyCode::Delete => {
                first_run.auth_input.delete();
                return OverlayAction::Keep;
            }
            KeyCode::Left => {
                first_run.auth_input.move_left();
                return OverlayAction::Keep;
            }
            KeyCode::Right => {
                first_run.auth_input.move_right();
                return OverlayAction::Keep;
            }
            KeyCode::Home => {
                first_run.auth_input.move_home();
                return OverlayAction::Keep;
            }
            KeyCode::End => {
                first_run.auth_input.move_end();
                return OverlayAction::Keep;
            }
            KeyCode::Char(ch) => {
                if !ch.is_control() {
                    first_run.auth_input.insert_char(ch);
                    return OverlayAction::Keep;
                }
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            return OverlayAction::Close;
        }
        KeyCode::Enter | KeyCode::Char('a') | KeyCode::Char('A') => {
            if snapshot.routes.is_empty() {
                let (overlay, scan_rx) = build_add_route_overlay();
                ui.target_scan = scan_rx;
                return OverlayAction::Replace(overlay);
            }
            return OverlayAction::Close;
        }
        KeyCode::Char('/') => {
            return OverlayAction::Replace(UiOverlay::CommandPalette(PaletteState::new()));
        }
        _ => {}
    }
    OverlayAction::Keep
}
