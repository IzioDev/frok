use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use super::super::actions::filter_actions;
use super::super::app_state::{AddRouteStep, ToastKind, UiSnapshot};
use super::super::theme::Theme;
use super::helpers::{
    add_route_hint, auth_status_label, gradient_spans, is_command_input, selected_mode_label,
    selected_target_label, step_label, truncate,
};
use super::layout::centered_rect;
use super::widgets::render_text_input;

pub(super) fn render_palette(
    frame: &mut ratatui::Frame,
    area: Rect,
    _snapshot: &UiSnapshot,
    theme: &Theme,
    state: &super::super::app_state::PaletteState,
) -> Option<(u16, u16)> {
    let block = centered_rect(area, 70, 60);
    frame.render_widget(Clear, block);

    let is_command = is_command_input(&state.input.value);
    let title = if is_command {
        "Command"
    } else {
        "Command palette"
    };

    let container = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));
    frame.render_widget(container.clone(), block);

    let inner = container.inner(block);
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let prompt = if is_command { ":" } else { ">" };
    let cursor = render_text_input(frame, rows[0], prompt, &state.input, theme, None);

    let mut actions = Vec::new();
    if !is_command {
        let filtered = filter_actions(&state.input.value);
        for action in filtered {
            actions.push(ListItem::new(Line::from(vec![
                Span::styled(action.label, Style::default().fg(theme.text)),
                Span::raw("  "),
                Span::styled(action.desc, Style::default().fg(theme.muted)),
            ])));
        }
    }

    if actions.is_empty() {
        actions.push(ListItem::new(Line::from(Span::styled(
            if is_command {
                "Type a command, e.g. :register"
            } else {
                "No matching actions"
            },
            Style::default().fg(theme.muted),
        ))));
    }

    let mut list_state = ListState::default();
    if !is_command && !actions.is_empty() {
        list_state.select(Some(state.selection.min(actions.len().saturating_sub(1))));
    }

    let list = List::new(actions)
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, rows[1], &mut list_state);

    let hint = if is_command {
        "enter to run | esc to close"
    } else {
        "type to search | enter to run | : for commands"
    };
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("hint: ", Style::default().fg(theme.muted)),
        Span::styled(hint, Style::default().fg(theme.text)),
    ]));
    frame.render_widget(footer, rows[2]);

    cursor
}

pub(super) fn render_add_route(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    state: &super::super::app_state::AddRouteState,
) -> Option<(u16, u16)> {
    let block = centered_rect(area, 74, 70);
    frame.render_widget(Clear, block);

    let container = Block::default()
        .title("Arm route")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));
    frame.render_widget(container.clone(), block);

    let inner = container.inner(block);
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);

    let step_line = Line::from(vec![
        Span::styled("Step ", Style::default().fg(theme.muted)),
        Span::styled(step_label(state.step), Style::default().fg(theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(step_line), rows[0]);

    let mut cursor = None;
    match state.step {
        AddRouteStep::Target => {
            cursor = render_add_route_target(frame, rows[1], theme, state);
        }
        AddRouteStep::Mode => {
            render_add_route_mode(frame, rows[1], theme, state);
        }
        AddRouteStep::Name => {
            cursor = render_add_route_name(frame, rows[1], snapshot, theme, state);
        }
        AddRouteStep::Confirm => {
            render_add_route_confirm(frame, rows[1], snapshot, theme, state);
        }
    }

    let hint_line = Line::from(vec![
        Span::styled("hint: ", Style::default().fg(theme.muted)),
        Span::styled(add_route_hint(state.step), Style::default().fg(theme.text)),
    ]);
    frame.render_widget(Paragraph::new(hint_line), rows[2]);

    cursor
}

fn render_add_route_target(
    frame: &mut ratatui::Frame,
    area: Rect,
    theme: &Theme,
    state: &super::super::app_state::AddRouteState,
) -> Option<(u16, u16)> {
    let mut items = Vec::new();
    for target in &state.targets {
        let (status, status_style) = if target.addr.is_empty() {
            ("manual", Style::default().fg(theme.muted))
        } else {
            match target.open {
                Some(true) => ("open", Style::default().fg(theme.good)),
                Some(false) => ("closed", Style::default().fg(theme.warn)),
                None => ("scan", Style::default().fg(theme.muted)),
            }
        };
        let line = Line::from(vec![
            Span::styled(&target.label, Style::default().fg(theme.text)),
            Span::raw("  "),
            Span::styled(status, status_style),
        ]);
        items.push(ListItem::new(line));
    }

    let mut stateful = ListState::default();
    if !state.targets.is_empty() {
        stateful.select(Some(
            state
                .target_selection
                .min(state.targets.len().saturating_sub(1)),
        ));
    }

    let block = Block::default()
        .title("Choose local target")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut stateful);

    if let Some(selected) = state.targets.get(state.target_selection) {
        if selected.addr.is_empty() {
            let input_area = Rect::new(
                area.x + 2,
                area.y + area.height.saturating_sub(2),
                area.width.saturating_sub(4),
                1,
            );
            return render_text_input(
                frame,
                input_area,
                "addr",
                &state.target_manual,
                theme,
                Some("127.0.0.1:3000"),
            );
        }
    }

    None
}

fn render_add_route_mode(
    frame: &mut ratatui::Frame,
    area: Rect,
    theme: &Theme,
    state: &super::super::app_state::AddRouteState,
) {
    let modes = ["HTTP", "HTTP2 (gRPC)", "TCP"];
    let mut items = Vec::new();
    for mode in modes {
        items.push(ListItem::new(Line::from(Span::styled(
            mode,
            Style::default().fg(theme.text),
        ))));
    }

    let mut list_state = ListState::default();
    list_state.select(Some(
        state.mode_selection.min(modes.len().saturating_sub(1)),
    ));

    let block = Block::default()
        .title("Choose mode")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_add_route_name(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    state: &super::super::app_state::AddRouteState,
) -> Option<(u16, u16)> {
    let block = Block::default()
        .title("Choose name / prefix")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let cursor = render_text_input(
        frame,
        rows[0],
        "name",
        &state.name_input,
        theme,
        Some("myapp"),
    );

    let name = if state.name_input.value.trim().is_empty() {
        "<name>"
    } else {
        state.name_input.value.trim()
    };

    let preview = format!("https://{name}.{}", snapshot.public_domain);
    let preview_line = Line::from(vec![
        Span::styled("preview ", Style::default().fg(theme.muted)),
        Span::styled(preview, Style::default().fg(theme.accent_alt)),
    ]);
    frame.render_widget(Paragraph::new(preview_line), rows[1]);

    if let Some(error) = &state.error {
        let err_line = Line::from(vec![
            Span::styled("error ", Style::default().fg(theme.danger)),
            Span::styled(error.as_str(), Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(err_line), rows[2]);
    }

    cursor
}

fn render_add_route_confirm(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    state: &super::super::app_state::AddRouteState,
) {
    let block = Block::default()
        .title("Arm route")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));

    let name = state.name_input.value.trim();
    let name = if name.is_empty() { "<name>" } else { name };

    let preview = format!("https://{name}.{}", snapshot.public_domain);
    let target = selected_target_label(state);
    let mode = selected_mode_label(state.mode_selection);

    let lines = vec![
        Line::from(vec![
            Span::styled("TARGET ", Style::default().fg(theme.muted)),
            Span::styled(target, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("MODE ", Style::default().fg(theme.muted)),
            Span::styled(mode, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("PUBLIC ", Style::default().fg(theme.muted)),
            Span::styled(preview, Style::default().fg(theme.accent_alt)),
        ]),
        Line::from(""),
        Line::from(gradient_spans(
            "ENTER TO ARM ROUTE.",
            theme.gradient_primary,
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(super) fn render_first_run(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    state: &super::super::app_state::FirstRunState,
) -> Option<(u16, u16)> {
    let block = centered_rect(area, 70, 60);
    frame.render_widget(Clear, block);

    let container = Block::default()
        .title("QUICKSTART")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.panel_border))
        .style(Style::default().bg(theme.panel));
    frame.render_widget(container.clone(), block);

    let inner = container.inner(block);
    if inner.height == 0 || inner.width == 0 {
        return None;
    }

    let mut content = vec![
        Line::from(vec![
            Span::styled("EDGE ", Style::default().fg(theme.muted)),
            Span::styled(&snapshot.edge, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("AUTH ", Style::default().fg(theme.muted)),
            Span::styled(
                auth_status_label(&snapshot.auth_status),
                Style::default().fg(theme.text),
            ),
        ]),
        Line::from(vec![
            Span::styled("AGENT ", Style::default().fg(theme.muted)),
            Span::styled(&snapshot.agent_label, Style::default().fg(theme.text)),
        ]),
    ];

    if let Some(auth_url) = &snapshot.auth_url {
        content.push(Line::from(""));
        content.push(Line::from(vec![
            Span::styled("LOGIN ", Style::default().fg(theme.muted)),
            Span::styled(
                truncate(auth_url, 52),
                Style::default().fg(theme.accent_alt),
            ),
        ]));
        content.push(Line::from(vec![
            Span::styled("hint ", Style::default().fg(theme.muted)),
            Span::styled("open URL to authenticate", Style::default().fg(theme.text)),
        ]));
    }

    content.push(Line::from(""));
    content.push(Line::from(vec![Span::styled(
        "NO ROUTES DETECTED.",
        Style::default().fg(theme.muted),
    )]));
    content.push(Line::from(""));
    if snapshot.auth_url.is_some() {
        content.push(Line::from(gradient_spans(
            "ENTER  SUBMIT CODE",
            theme.gradient_primary,
            Style::default().add_modifier(Modifier::BOLD),
        )));
    } else {
        content.push(Line::from(gradient_spans(
            "A / ENTER  ARM ROUTE",
            theme.gradient_primary,
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    content.push(Line::from(vec![
        Span::styled("/", Style::default().fg(theme.accent)),
        Span::raw(" command palette"),
    ]));
    content.push(Line::from(vec![
        Span::styled("ESC", Style::default().fg(theme.accent)),
        Span::raw(" dismiss"),
    ]));

    let mut cursor = None;
    if snapshot.auth_url.is_some() {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let paragraph = Paragraph::new(Text::from(content))
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, rows[0]);
        cursor = render_text_input(
            frame,
            rows[1],
            "code",
            &state.auth_input,
            theme,
            Some("paste code or URL"),
        );
    } else {
        let paragraph = Paragraph::new(Text::from(content))
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: true });
        frame.render_widget(paragraph, inner);
    }

    cursor
}

pub(super) fn render_toast(
    frame: &mut ratatui::Frame,
    area: Rect,
    snapshot: &UiSnapshot,
    theme: &Theme,
    now: std::time::Instant,
) {
    let Some(toast) = &snapshot.toast else {
        return;
    };
    if now.duration_since(toast.created_at) > std::time::Duration::from_secs(3) {
        return;
    }

    let width = area.width.min(80).max(10);
    let height = 3u16.min(area.height);
    if height == 0 {
        return;
    }
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height.saturating_sub(height);
    let rect = Rect::new(x, y, width, height);

    let color = match toast.kind {
        ToastKind::Info => theme.accent,
        ToastKind::Warn => theme.warn,
        ToastKind::Error => theme.danger,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(theme.panel));

    let text = Paragraph::new(Line::from(Span::styled(
        toast.message.as_str(),
        Style::default().fg(theme.text),
    )))
    .alignment(ratatui::layout::Alignment::Center)
    .block(block);

    frame.render_widget(Clear, rect);
    frame.render_widget(text, rect);
}
