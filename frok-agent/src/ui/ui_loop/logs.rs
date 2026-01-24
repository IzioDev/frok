use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;

use super::super::app_state::{LogsFilter, ToastKind, UiSnapshot, UiState};
use super::super::clipboard::copy_to_clipboard;
use super::super::commands::Command;
use super::super::logs::filter_logs;

pub(super) fn handle_logs_key(
    key: KeyEvent,
    ui: &mut UiState,
    snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
    _tx: &UnboundedSender<Command>,
) -> bool {
    if ui.logs.search_active {
        return handle_logs_search_key(key, ui, snapshot, state);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('l') | KeyCode::Char('L') => {
            state.hide_logs_blocking();
        }
        KeyCode::Char('/') => {
            ui.logs.search_active = true;
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            ui.logs.filter = LogsFilter::All;
            ui.logs.selected = 0;
            ui.logs.scroll = 0;
        }
        KeyCode::Char('v') | KeyCode::Char('V') => {
            ui.logs.filter = LogsFilter::Events;
            ui.logs.selected = 0;
            ui.logs.scroll = 0;
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            ui.logs.filter = LogsFilter::Errors;
            ui.logs.selected = 0;
            ui.logs.scroll = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if ui.logs.selected > 0 {
                ui.logs.selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let total = filtered_log_len(snapshot, ui);
            if ui.logs.selected + 1 < total {
                ui.logs.selected += 1;
            }
        }
        KeyCode::PageUp => {
            let total = filtered_log_len(snapshot, ui);
            let jump = log_view_capacity().min(total.max(1));
            ui.logs.selected = ui.logs.selected.saturating_sub(jump);
        }
        KeyCode::PageDown => {
            let total = filtered_log_len(snapshot, ui);
            let jump = log_view_capacity().min(total.max(1));
            ui.logs.selected = (ui.logs.selected + jump).min(total.saturating_sub(1));
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(line) = selected_log_line(snapshot, ui) {
                match copy_to_clipboard(&line) {
                    Ok(()) => ui.set_toast("LINE COPIED.", ToastKind::Info),
                    Err(err) => ui.set_toast(format!("Copy failed: {err}"), ToastKind::Error),
                }
            }
        }
        _ => {}
    }

    ensure_log_visible(ui, snapshot);
    false
}

fn handle_logs_search_key(
    key: KeyEvent,
    ui: &mut UiState,
    _snapshot: &UiSnapshot,
    state: &Arc<super::super::AppState>,
) -> bool {
    match key.code {
        KeyCode::Esc => {
            ui.logs.search_active = false;
            ui.logs.search.clear();
        }
        KeyCode::Enter => {
            ui.logs.search_active = false;
            ui.logs.selected = 0;
            ui.logs.scroll = 0;
        }
        KeyCode::Backspace => {
            ui.logs.search.backspace();
        }
        KeyCode::Delete => {
            ui.logs.search.delete();
        }
        KeyCode::Left => {
            ui.logs.search.move_left();
        }
        KeyCode::Right => {
            ui.logs.search.move_right();
        }
        KeyCode::Home => {
            ui.logs.search.move_home();
        }
        KeyCode::End => {
            ui.logs.search.move_end();
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return false;
            }
            ui.logs.search.insert_char(c);
        }
        _ => {}
    }

    state.show_logs_blocking();
    false
}

fn filtered_log_len(snapshot: &UiSnapshot, ui: &UiState) -> usize {
    filter_logs(&snapshot.logs, ui.logs.filter, &ui.logs.search.value).len()
}

fn selected_log_line(snapshot: &UiSnapshot, ui: &UiState) -> Option<String> {
    let filtered = filter_logs(&snapshot.logs, ui.logs.filter, &ui.logs.search.value);
    filtered
        .get(ui.logs.selected)
        .map(|line| line.as_ref().to_string())
}

fn ensure_log_visible(ui: &mut UiState, snapshot: &UiSnapshot) {
    let total = filtered_log_len(snapshot, ui);
    if total == 0 {
        ui.logs.selected = 0;
        ui.logs.scroll = 0;
        return;
    }

    if ui.logs.selected >= total {
        ui.logs.selected = total - 1;
    }

    let capacity = log_view_capacity();
    if ui.logs.selected < ui.logs.scroll {
        ui.logs.scroll = ui.logs.selected;
    }
    if ui.logs.selected >= ui.logs.scroll + capacity {
        ui.logs.scroll = ui.logs.selected + 1 - capacity;
    }

    if ui.logs.scroll + capacity > total {
        ui.logs.scroll = total.saturating_sub(capacity);
    }
}

fn log_view_capacity() -> usize {
    match crossterm::terminal::size() {
        Ok((_, height)) => height.saturating_sub(6) as usize,
        Err(_) => 10,
    }
    .max(1)
}
