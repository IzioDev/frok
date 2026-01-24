use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::app_state::{FirstRunState, PaletteState, ToastKind, UiOverlay, UiSnapshot, UiState};
use super::commands::Command;
use super::render::render_ui;
use crate::config;

mod add_route;
mod dashboard;
mod first_run;
mod logs;
mod palette;

pub fn start_ui(
    state: Arc<super::AppState>,
    tx: UnboundedSender<Command>,
    token: CancellationToken,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        if enable_raw_mode().is_err() {
            return;
        }
        let mut stdout = std::io::stdout();
        if execute!(stdout, EnterAlternateScreen).is_err() {
            let _ = disable_raw_mode();
            return;
        }

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(_) => {
                let _ = disable_raw_mode();
                return;
            }
        };

        let mut ui = UiState::default();
        let prefs = config::load_ui_prefs();
        ui.theme_kind = prefs.theme;
        ui.texture = prefs.texture;
        ui.first_run_seen = prefs.quickstart_seen;

        loop {
            if token.is_cancelled() {
                break;
            }

            if let Some(rx) = &ui.target_scan {
                if let Ok(scanned) = rx.try_recv() {
                    if let UiOverlay::AddRoute(ref mut wizard) = ui.overlay {
                        let selection = wizard.target_selection;
                        wizard.targets = scanned;
                        if !wizard.targets.is_empty() {
                            wizard.target_selection =
                                selection.min(wizard.targets.len().saturating_sub(1));
                        }
                    }
                    ui.target_scan = None;
                }
            }

            let mut snapshot = state.snapshot(&ui);
            ui.clamp_route_index(snapshot.routes.len());
            let mut refreshed = false;
            if ui.route_index != snapshot.route_index {
                refreshed = true;
            }
            if !ui.first_run_seen
                && snapshot.routes.is_empty()
                && matches!(ui.overlay, UiOverlay::None)
            {
                ui.overlay = UiOverlay::FirstRun(FirstRunState::new());
                ui.first_run_seen = true;
                if let Err(err) = config::save_quickstart_seen(true) {
                    ui.set_toast(format!("prefs save failed: {err}"), ToastKind::Error);
                }
                refreshed = true;
            }
            if refreshed {
                snapshot = state.snapshot(&ui);
            }

            let now = Instant::now();
            let mut theme = super::theme::Theme::from_kind(ui.theme_kind);
            theme.texture = ui.texture;

            let _ = terminal.draw(|frame| {
                render_ui(frame, &snapshot, &theme, now);
            });

            if event::poll(Duration::from_millis(60)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    if handle_key_event(key, &mut ui, &snapshot, &state, &tx) {
                        break;
                    }
                }
            }
        }

        let _ = disable_raw_mode();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        let _ = terminal.show_cursor();
    })
}

#[derive(Debug, Clone)]
enum OverlayAction {
    Keep,
    Close,
    Replace(UiOverlay),
    Quit,
}

fn handle_key_event(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::AppState>,
    tx: &UnboundedSender<Command>,
) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let _ = tx.send(Command::Quit);
        return true;
    }

    if !matches!(ui.overlay, UiOverlay::None) {
        let mut overlay = std::mem::replace(&mut ui.overlay, UiOverlay::None);
        let action = match &mut overlay {
            UiOverlay::CommandPalette(palette) => {
                palette::handle_palette_key(key, ui, snapshot, state, tx, palette)
            }
            UiOverlay::AddRoute(wizard) => {
                add_route::handle_add_route_key(key, ui, snapshot, state, tx, wizard)
            }
            UiOverlay::FirstRun(first_run) => {
                first_run::handle_first_run_key(key, ui, snapshot, state, first_run)
            }
            UiOverlay::None => OverlayAction::Keep,
        };

        return match action {
            OverlayAction::Keep => {
                ui.overlay = overlay;
                false
            }
            OverlayAction::Close => {
                ui.overlay = UiOverlay::None;
                false
            }
            OverlayAction::Replace(new_overlay) => {
                ui.overlay = new_overlay;
                false
            }
            OverlayAction::Quit => {
                ui.overlay = UiOverlay::None;
                true
            }
        };
    }

    if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
        ui.overlay = UiOverlay::CommandPalette(PaletteState::new());
        return false;
    }

    if key.code == KeyCode::Char('?') {
        ui.overlay = UiOverlay::FirstRun(FirstRunState::new());
        if !ui.first_run_seen {
            ui.first_run_seen = true;
            if let Err(err) = config::save_quickstart_seen(true) {
                ui.set_toast(format!("prefs save failed: {err}"), ToastKind::Error);
            }
        }
        return false;
    }

    if snapshot.is_logs() {
        logs::handle_logs_key(key, ui, snapshot, state, tx)
    } else {
        dashboard::handle_dashboard_key(key, ui, snapshot, state, tx)
    }
}
