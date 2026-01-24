use std::borrow::Cow;
use std::collections::VecDeque;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use super::super::app_state::{LogsFilter, UiSnapshot};
use super::super::input::slice_visible;
use super::super::logs::{filter_logs, is_error_line};
use super::super::theme::Theme;
use super::helpers::filter_style;

pub(super) fn render_logs(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) {
    let filtered = filter_logs(
        &snapshot.logs,
        snapshot.logs_state.filter,
        &snapshot.logs_state.search.value,
    );

    let header_height = 2u16;
    let footer_height = 1u16;
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .split(area);

    let header = render_logs_header(snapshot, theme);
    frame.render_widget(header, inner[0]);

    let max_visible = inner[1].height.saturating_sub(2) as usize;
    let max_visible = max_visible.max(1);

    let start = snapshot.logs_state.scroll.min(filtered.len());
    let end = (start + max_visible).min(filtered.len());
    let visible = &filtered[start..end];

    let mut items = Vec::new();
    for line in visible {
        let style = log_style(line.as_ref(), theme);
        items.push(ListItem::new(Line::from(Span::styled(
            (*line).clone(),
            style,
        ))));
    }

    let mut state = ListState::default();
    if !visible.is_empty() {
        let selected = snapshot.logs_state.selected.saturating_sub(start);
        if selected < visible.len() {
            state.select(Some(selected));
        }
    }

    let block = Block::default()
        .title("Signal stream")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_widget(Clear, inner[1]);
    frame.render_stateful_widget(list, inner[1], &mut state);

    let footer = render_logs_footer(snapshot, theme);
    frame.render_widget(footer, inner[2]);
}

fn render_logs_header(snapshot: &UiSnapshot, theme: &Theme) -> Paragraph<'static> {
    let filter = snapshot.logs_state.filter;
    let (all_style, event_style, error_style) = (
        filter_style(filter == LogsFilter::All, theme),
        filter_style(filter == LogsFilter::Events, theme),
        filter_style(filter == LogsFilter::Errors, theme),
    );

    let search = if snapshot.logs_state.search.value.is_empty() {
        "(none)".to_string()
    } else {
        snapshot.logs_state.search.value.clone()
    };

    let line = Line::from(vec![
        Span::styled("FILTER ", Style::default().fg(theme.muted)),
        Span::styled("[A] All", all_style),
        Span::raw("  "),
        Span::styled("[V] Events", event_style),
        Span::raw("  "),
        Span::styled("[E] Errors", error_style),
        Span::raw("  "),
        Span::styled("/", Style::default().fg(theme.accent)),
        Span::raw(" search: "),
        Span::styled(search, Style::default().fg(theme.text)),
    ]);

    Paragraph::new(Text::from(vec![line])).style(Style::default().fg(theme.text))
}

fn render_logs_footer(snapshot: &UiSnapshot, theme: &Theme) -> Paragraph<'static> {
    let hint = if snapshot.logs_state.search_active {
        "typing search | enter to apply | esc to cancel"
    } else {
        "j/k or arrows to move | y copy line | l to exit"
    };
    Paragraph::new(Line::from(vec![
        Span::styled("hint: ", Style::default().fg(theme.muted)),
        Span::styled(hint, Style::default().fg(theme.text)),
    ]))
}

pub(super) fn render_logs_search_cursor(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) -> Option<(u16, u16)> {
    let header_height = 2u16;
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let line = Line::from(vec![
        Span::styled("search: ", Style::default().fg(theme.muted)),
        Span::styled(
            &snapshot.logs_state.search.value,
            Style::default().fg(theme.text),
        ),
    ]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, inner[0]);

    let width = inner[0].width.saturating_sub(8) as usize;
    let (_, cursor_visible, _) = slice_visible(
        &snapshot.logs_state.search.value,
        snapshot.logs_state.search.cursor,
        width,
    );
    let cursor_x = inner[0].x + 8 + cursor_visible as u16;
    let cursor_y = inner[0].y;
    Some((cursor_x, cursor_y))
}

pub(super) fn build_log_items(
    logs: &VecDeque<Cow<'static, str>>,
    max_render: usize,
    max_visible: usize,
    theme: &Theme,
) -> Vec<ListItem<'static>> {
    if logs.is_empty() || max_visible == 0 {
        return Vec::new();
    }

    let mut recent = logs.iter().rev().take(max_render).collect::<Vec<_>>();
    recent.reverse();
    let visible = max_visible.min(recent.len());
    let start = recent.len().saturating_sub(visible);

    recent[start..]
        .iter()
        .map(|line| {
            let style = log_style(line.as_ref(), theme);
            ListItem::new(Line::from(Span::styled((*line).clone(), style)))
        })
        .collect()
}

fn log_style(line: &str, theme: &Theme) -> Style {
    let lower = line.to_ascii_lowercase();
    if is_error_line(line) {
        Style::default().fg(theme.danger)
    } else if lower.contains("warn") {
        Style::default().fg(theme.warn)
    } else if lower.contains("connected") || lower.contains("registered") {
        Style::default().fg(theme.good)
    } else {
        Style::default().fg(theme.text)
    }
}
