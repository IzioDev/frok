use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use frok_protocol::IngressMode;

use super::super::app_state::{RouteEntry, UiSnapshot};
use super::super::theme::Theme;
use super::helpers::{gradient_spans, route_status_color, route_status_label, style_with_bg};
use super::logs::build_log_items;

const MAX_LOG_RENDER: usize = 200;

pub(super) fn render_dashboard(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let min_main = 6u16;
    let mut log_height = if area.height >= 18 { 6u16 } else { 4u16 };
    if area.height < min_main + log_height {
        log_height = area.height.saturating_sub(min_main);
    }
    if log_height < 3 {
        log_height = 0;
    }

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(min_main), Constraint::Length(log_height)])
        .split(area);

    let main = body[0];
    let logs_area = body[1];

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(main);

    render_routes_list(frame, columns[0], snapshot, theme);
    render_route_details(frame, columns[1], snapshot, theme);

    if log_height > 0 {
        render_signal_preview(frame, logs_area, snapshot, theme);
    }
}

fn render_routes_list(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) {
    let mut items = Vec::new();

    for (idx, route) in snapshot.routes.iter().enumerate() {
        let status = route_status_label(route.status);
        let status_color = route_status_color(route.status, theme);
        let selected = idx == snapshot.route_index;
        let selection_bg = if selected {
            Some(theme.selection_bg)
        } else {
            None
        };

        let mut line1_spans = Vec::new();
        if selected {
            line1_spans.extend(gradient_spans(
                ">> ",
                theme.gradient_primary,
                Style::default()
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            line1_spans.extend(gradient_spans(
                &route.name,
                theme.gradient_primary,
                Style::default()
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            line1_spans.push(Span::styled("   ", Style::default()));
            line1_spans.push(Span::styled(
                &route.name,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        line1_spans.push(Span::styled(
            "  ",
            style_with_bg(Style::default(), selection_bg),
        ));
        line1_spans.push(Span::styled(
            status,
            style_with_bg(Style::default().fg(status_color), selection_bg),
        ));
        let line1 = Line::from(line1_spans);

        let prefix_style = style_with_bg(Style::default().fg(theme.muted), selection_bg);
        let mut line2_spans = Vec::new();
        line2_spans.push(Span::styled("   ", prefix_style));
        line2_spans.push(Span::styled(
            &route.local_addr,
            style_with_bg(Style::default().fg(theme.text), selection_bg),
        ));
        line2_spans.push(Span::styled(
            "  ",
            style_with_bg(Style::default(), selection_bg),
        ));
        line2_spans.push(Span::styled(
            route.mode.label(),
            style_with_bg(Style::default().fg(theme.muted), selection_bg),
        ));
        let line2 = Line::from(line2_spans);

        items.push(ListItem::new(Text::from(vec![line1, line2])));
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "NO ROUTES. PRESS A TO ARM ONE.",
            Style::default().fg(theme.muted),
        ))));
    }

    let mut state = ListState::default();
    if !snapshot.routes.is_empty() {
        state.select(Some(snapshot.route_index));
    }

    let block = Block::default()
        .title(format!("Routes ({})", snapshot.routes.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let list = List::new(items)
        .block(block)
        .highlight_symbol("")
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(Clear, area);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_route_details(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) {
    let mut lines = Vec::new();

    if let Some(route) = snapshot.selected_route() {
        lines.push(Line::from(vec![Span::styled(
            "PUBLIC LINK",
            Style::default().fg(theme.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            route.public_url.clone(),
            Style::default()
                .fg(theme.accent_alt)
                .add_modifier(Modifier::BOLD),
        )]));
        let cli_command = build_cli_command(&route);
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "CLI",
            Style::default().fg(theme.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            cli_command,
            Style::default().fg(theme.text),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("LOCAL", Style::default().fg(theme.muted)),
            Span::raw("  "),
            Span::styled(route.local_addr, Style::default().fg(theme.text)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("MODE", Style::default().fg(theme.muted)),
            Span::raw("  "),
            Span::styled(route.mode.label(), Style::default().fg(theme.text)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("STATUS", Style::default().fg(theme.muted)),
            Span::raw("  "),
            Span::styled(
                route_status_label(route.status),
                Style::default().fg(route_status_color(route.status, theme)),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("[Y]", Style::default().fg(theme.accent)),
            Span::raw(" copy  "),
            Span::styled("[O]", Style::default().fg(theme.accent)),
            Span::raw(" open  "),
            Span::styled("[D]", Style::default().fg(theme.accent)),
            Span::raw(" delete"),
        ]));
    } else {
        lines.push(Line::from(vec![Span::styled(
            "NO ROUTE SELECTED.",
            Style::default().fg(theme.muted),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("[A]", Style::default().fg(theme.accent)),
            Span::raw(" arm route"),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[?]", Style::default().fg(theme.accent)),
        Span::styled(" quickstart", Style::default().fg(theme.muted)),
    ]));

    let block = Block::default()
        .title("Route details")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let panel = Paragraph::new(Text::from(lines))
        .block(block)
        .style(Style::default().fg(theme.text))
        .wrap(Wrap { trim: true });

    frame.render_widget(panel, area);
}

fn render_signal_preview(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
) {
    let max_visible = area.height.saturating_sub(2) as usize;
    let visible = 5usize.min(max_visible);
    let logs = build_log_items(&snapshot.logs, MAX_LOG_RENDER, visible, theme);

    let block = Block::default()
        .title("Signal / recent")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    frame.render_widget(List::new(logs).block(block), area);
}

fn build_cli_command(route: &RouteEntry) -> String {
    if let Some((host, port)) = split_host_port(&route.local_addr) {
        if is_loopback_host(&host) {
            return match route.mode {
                IngressMode::Tcp => format!("frok tcp {port} --name {}", route.name),
                IngressMode::Http2 => {
                    format!("frok http {port} --name {} --mode http2", route.name)
                }
                IngressMode::Http1 => format!("frok http {port} --name {}", route.name),
            };
        }
    }

    let mode = match route.mode {
        IngressMode::Tcp => "tcp",
        IngressMode::Http2 => "http2",
        IngressMode::Http1 => "http",
    };
    format!("frok register {} {} {}", route.name, route.local_addr, mode)
}

fn split_host_port(input: &str) -> Option<(String, u16)> {
    if let Some(rest) = input.strip_prefix('[') {
        let (host, remainder) = rest.split_once("]")?;
        let port = remainder.strip_prefix(':')?;
        return port.parse::<u16>().ok().map(|p| (host.to_string(), p));
    }

    let (host, port) = input.rsplit_once(':')?;
    port.parse::<u16>().ok().map(|p| (host.to_string(), p))
}

fn is_loopback_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "127.0.0.1" | "localhost" | "0.0.0.0" | "::1" | "::"
    )
}
