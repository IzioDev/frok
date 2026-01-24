use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

use super::super::actions::{PaletteActionId, filter_actions};
use super::super::app_state::{PaletteState, ToastKind, UiSnapshot, UiState};
use super::super::clipboard::copy_to_clipboard;
use super::super::commands::{Command, parse_command};
use super::super::theme::{Texture, ThemeKind};
use super::add_route::build_add_route_overlay;
use crate::build_info;
use crate::config;

use super::OverlayAction;

pub(super) fn handle_palette_key(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
    tx: &UnboundedSender<Command>,
    palette: &mut PaletteState,
) -> OverlayAction {
    match key.code {
        KeyCode::Esc => {
            return OverlayAction::Close;
        }
        KeyCode::Enter => {
            if is_command_input(&palette.input.value) {
                let cmd = palette.input.value.clone();
                if let Some(parsed) = parse_command(&cmd) {
                    let _ = tx.send(parsed);
                } else {
                    ui.set_toast("Unknown command", ToastKind::Warn);
                }
                return OverlayAction::Close;
            }

            let actions = filter_actions(&palette.input.value);
            if let Some(action) = actions.get(palette.selection) {
                return run_palette_action(action.id, ui, snapshot, state, tx);
            }
        }
        KeyCode::Up => {
            palette.selection = palette.selection.saturating_sub(1);
        }
        KeyCode::Down => {
            let actions = filter_actions(&palette.input.value);
            if !actions.is_empty() {
                palette.selection = (palette.selection + 1).min(actions.len() - 1);
            }
        }
        KeyCode::Backspace => {
            palette.input.backspace();
            palette.selection = 0;
        }
        KeyCode::Delete => {
            palette.input.delete();
        }
        KeyCode::Left => {
            palette.input.move_left();
        }
        KeyCode::Right => {
            palette.input.move_right();
        }
        KeyCode::Home => {
            palette.input.move_home();
        }
        KeyCode::End => {
            palette.input.move_end();
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return OverlayAction::Keep;
            }
            palette.input.insert_char(c);
            palette.selection = 0;
        }
        _ => {}
    }

    OverlayAction::Keep
}

fn run_palette_action(
    action: PaletteActionId,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
    tx: &UnboundedSender<Command>,
) -> OverlayAction {
    match action {
        PaletteActionId::AddRoute => {
            let (overlay, scan_rx) = build_add_route_overlay();
            ui.target_scan = scan_rx;
            OverlayAction::Replace(overlay)
        }
        PaletteActionId::DeleteRoute => {
            if let Some(route) = snapshot.selected_route() {
                let _ = tx.send(Command::Unregister { name: route.name });
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
            OverlayAction::Close
        }
        PaletteActionId::CopyUrl => {
            if let Some(route) = snapshot.selected_route() {
                match copy_to_clipboard(&route.public_url) {
                    Ok(()) => ui.set_toast("LINK COPIED.", ToastKind::Info),
                    Err(err) => ui.set_toast(format!("Copy failed: {err}"), ToastKind::Error),
                }
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
            OverlayAction::Close
        }
        PaletteActionId::OpenUrl => {
            if let Some(route) = snapshot.selected_route() {
                if let Err(err) = webbrowser::open(&route.public_url) {
                    ui.set_toast(format!("Open failed: {err}"), ToastKind::Error);
                }
            } else {
                ui.set_toast("NO ROUTE SELECTED.", ToastKind::Warn);
            }
            OverlayAction::Close
        }
        PaletteActionId::ShowLogs => {
            state.show_logs_blocking();
            OverlayAction::Close
        }
        PaletteActionId::Connect => {
            let _ = tx.send(Command::Connect);
            OverlayAction::Close
        }
        PaletteActionId::ClearLogs => {
            let _ = tx.send(Command::ClearLogs);
            OverlayAction::Close
        }
        PaletteActionId::OpenGithub => {
            if let Err(err) = webbrowser::open(build_info::GITHUB_URL) {
                ui.set_toast(format!("Open failed: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::ThemeNeonSunset => {
            ui.theme_kind = ThemeKind::NeonSunset;
            ui.set_toast("THEME: NEON SUNSET.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::ThemeAcidMint => {
            ui.theme_kind = ThemeKind::AcidMint;
            ui.set_toast("THEME: ACID MINT.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::ThemeInfrared => {
            ui.theme_kind = ThemeKind::Infrared;
            ui.set_toast("THEME: INFRARED.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::TextureNone => {
            ui.texture = Texture::None;
            ui.set_toast("TEXTURE: NONE.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::TextureScanlines => {
            ui.texture = Texture::Scanlines;
            ui.set_toast("TEXTURE: SCANLINES.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Error);
            }
            OverlayAction::Close
        }
        PaletteActionId::TextureGrid => {
            ui.texture = Texture::Grid;
            ui.set_toast("TEXTURE: GRID.", ToastKind::Info);
            if let Err(err) = config::save_ui_prefs(ui.theme_kind, ui.texture) {
                ui.set_toast(format!("SAVE FAILED: {err}"), ToastKind::Warn);
            }
            OverlayAction::Close
        }
        PaletteActionId::Help => {
            let _ = tx.send(Command::Help);
            OverlayAction::Close
        }
        PaletteActionId::Quit => {
            let _ = tx.send(Command::Quit);
            OverlayAction::Quit
        }
    }
}

fn is_command_input(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with(':') || trimmed.starts_with('>')
}
