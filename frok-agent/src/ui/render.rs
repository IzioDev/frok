use std::time::Instant;

use ratatui::{style::Style, widgets::Block};

use super::app_state::{UiOverlay, UiSnapshot};
use super::theme::Theme;

mod dashboard;
mod header;
mod helpers;
mod layout;
mod logs;
mod overlays;
mod texture;
mod widgets;

pub(crate) fn render_ui(
    frame: &mut ratatui::Frame,
    snapshot: &UiSnapshot,
    theme: &Theme,
    now: Instant,
) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);
    texture::render_texture(frame, area, theme);

    let (header_area, body_area) = layout::split_main(area);
    if let Some(header_area) = header_area {
        header::render_header(frame, header_area, snapshot, theme, now);
    }

    if snapshot.is_logs() {
        logs::render_logs(frame, body_area, snapshot, theme);
    } else {
        dashboard::render_dashboard(frame, body_area, snapshot, theme);
    }

    let mut cursor = None;
    match &snapshot.overlay {
        UiOverlay::CommandPalette(state) => {
            cursor = overlays::render_palette(frame, area, snapshot, theme, state);
        }
        UiOverlay::AddRoute(state) => {
            cursor = overlays::render_add_route(frame, area, snapshot, theme, state);
        }
        UiOverlay::FirstRun(state) => {
            cursor = overlays::render_first_run(frame, area, snapshot, theme, state);
        }
        UiOverlay::None => {}
    }

    if cursor.is_none() && snapshot.is_logs() && snapshot.logs_state.search_active {
        cursor = logs::render_logs_search_cursor(frame, body_area, snapshot, theme);
    }

    overlays::render_toast(frame, area, snapshot, theme, now);

    if let Some(pos) = cursor {
        frame.set_cursor_position(pos);
    }
}
