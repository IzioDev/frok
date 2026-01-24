use std::time::Instant;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

use super::super::app_state::UiSnapshot;
use super::super::theme::Theme;
use super::helpers::{auth_status_label, format_duration, gradient_spans, status_color};
use crate::build_info;

pub(super) fn render_header(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    now: Instant,
) {
    let uptime = format_duration(now.duration_since(snapshot.started_at));
    let routes = snapshot.routes.len();
    let log_count = snapshot.logs.len();
    let mode = if snapshot.is_logs() {
        "logs"
    } else {
        "dashboard"
    };

    let status_color = status_color(&snapshot.status, theme);
    let auth_line = auth_status_label(&snapshot.auth_status);
    let agent_build = build_info::format_build(&snapshot.agent_build);
    let edge_build = snapshot
        .edge_build
        .as_ref()
        .map(build_info::format_build)
        .unwrap_or_else(|| "unknown".to_string());

    let mut title_spans = gradient_spans(
        "FROK",
        theme.gradient_primary,
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED),
    );
    title_spans.push(Span::raw("  "));
    title_spans.push(Span::styled(
        "MAKE LOCAL PUBLIC.",
        Style::default()
            .fg(theme.muted)
            .add_modifier(Modifier::BOLD),
    ));
    let title = Line::from(title_spans);

    let status_line = Line::from(vec![
        Span::styled("LINK ", Style::default().fg(theme.muted)),
        Span::styled(&snapshot.status, Style::default().fg(status_color)),
        Span::styled(" | EDGE ", Style::default().fg(theme.muted)),
        Span::styled(&snapshot.edge, Style::default().fg(theme.text)),
        Span::styled(" | AUTH ", Style::default().fg(theme.muted)),
        Span::styled(auth_line, Style::default().fg(theme.accent)),
        Span::styled(" | VER ", Style::default().fg(theme.muted)),
        Span::styled(agent_build, Style::default().fg(theme.text)),
        Span::styled(" | EDGE VER ", Style::default().fg(theme.muted)),
        Span::styled(edge_build, Style::default().fg(theme.text)),
        Span::styled(" | ", Style::default().fg(theme.muted)),
        Span::styled(
            "ALPHA: EXPECT BUGS",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ),
    ]);

    let meta_line = Line::from(vec![
        Span::styled("AGENT ", Style::default().fg(theme.muted)),
        Span::styled(&snapshot.agent_label, Style::default().fg(theme.text)),
        Span::styled(" | ROUTES ", Style::default().fg(theme.muted)),
        Span::styled(routes.to_string(), Style::default().fg(theme.text)),
        Span::styled(" | SIGNAL ", Style::default().fg(theme.muted)),
        Span::styled(log_count.to_string(), Style::default().fg(theme.text)),
        Span::styled(" | VIEW ", Style::default().fg(theme.muted)),
        Span::styled(mode, Style::default().fg(theme.accent_alt)),
        Span::styled(" | UP ", Style::default().fg(theme.muted)),
        Span::styled(uptime, Style::default().fg(theme.text)),
    ]);

    let mut lines = vec![title, status_line, meta_line];
    let max_lines = area.height as usize;
    lines.truncate(max_lines);

    let header = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(theme.panel_border)),
        )
        .style(Style::default().fg(theme.text));

    frame.render_widget(header, area);
}
