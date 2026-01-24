use std::time::Duration;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::super::app_state::{AddRouteState, AddRouteStep, AuthStatus, RouteStatus};
use super::super::theme::Theme;

pub(super) fn gradient_spans(text: &str, gradient: [Color; 3], base: Style) -> Vec<Span<'static>> {
    let len = text.chars().count().max(1);
    text.chars()
        .enumerate()
        .map(|(idx, ch)| {
            let color = gradient_color(gradient, idx, len);
            Span::styled(ch.to_string(), base.fg(color))
        })
        .collect()
}

fn gradient_color(gradient: [Color; 3], idx: usize, len: usize) -> Color {
    if len <= 1 {
        return gradient[0];
    }

    let t = idx as f32 / (len.saturating_sub(1)) as f32;
    let (start, end, local) = if t <= 0.5 {
        (gradient[0], gradient[1], t / 0.5)
    } else {
        (gradient[1], gradient[2], (t - 0.5) / 0.5)
    };
    lerp_color(start, end, local)
}

fn lerp_color(start: Color, end: Color, t: f32) -> Color {
    let (sr, sg, sb) = color_to_rgb(start);
    let (er, eg, eb) = color_to_rgb(end);
    let lerp = |a: u8, b: u8| -> u8 {
        let a = a as f32;
        let b = b as f32;
        (a + (b - a) * t).round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(sr, er), lerp(sg, eg), lerp(sb, eb))
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

pub(super) fn style_with_bg(style: Style, bg: Option<Color>) -> Style {
    if let Some(color) = bg {
        style.bg(color)
    } else {
        style
    }
}

pub(super) fn status_color(status: &str, theme: &Theme) -> Color {
    let status_lower = status.to_ascii_lowercase();
    if status_lower.contains("disconnected") {
        theme.danger
    } else if status_lower.contains("connecting") {
        theme.warn
    } else if status_lower.contains("connected") {
        theme.good
    } else {
        theme.muted
    }
}

pub(super) fn auth_status_label(status: &AuthStatus) -> String {
    match status {
        AuthStatus::Unknown => "offline".to_string(),
        AuthStatus::Pending => "pending".to_string(),
        AuthStatus::Authenticated { subject } => format!("ok ({subject})"),
        AuthStatus::Failed { reason } => format!("failed ({})", truncate(reason, 24)),
    }
}

pub(super) fn route_status_label(status: RouteStatus) -> &'static str {
    match status {
        RouteStatus::Pending => "pending",
        RouteStatus::Live => "live",
        RouteStatus::Error => "error",
    }
}

pub(super) fn route_status_color(status: RouteStatus, theme: &Theme) -> Color {
    match status {
        RouteStatus::Pending => theme.warn,
        RouteStatus::Live => theme.good,
        RouteStatus::Error => theme.danger,
    }
}

pub(super) fn filter_style(active: bool, theme: &Theme) -> Style {
    if active {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted)
    }
}

pub(super) fn step_label(step: AddRouteStep) -> &'static str {
    match step {
        AddRouteStep::Target => "1/4 target",
        AddRouteStep::Mode => "2/4 mode",
        AddRouteStep::Name => "3/4 name",
        AddRouteStep::Confirm => "4/4 confirm",
    }
}

pub(super) fn add_route_hint(step: AddRouteStep) -> &'static str {
    match step {
        AddRouteStep::Target => "up/down to choose | enter to confirm | esc to cancel",
        AddRouteStep::Mode => "up/down to choose | enter to confirm | esc to cancel",
        AddRouteStep::Name => "type name | enter to continue | esc to cancel",
        AddRouteStep::Confirm => "enter to arm | esc to cancel",
    }
}

pub(super) fn selected_target_label(state: &AddRouteState) -> String {
    state
        .targets
        .get(state.target_selection)
        .map(|target| {
            if target.addr.is_empty() {
                if state.target_manual.value.is_empty() {
                    "manual".to_string()
                } else {
                    state.target_manual.value.clone()
                }
            } else {
                target.label.clone()
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn selected_mode_label(selection: usize) -> &'static str {
    match selection {
        1 => "http2",
        2 => "tcp",
        _ => "http",
    }
}

pub(super) fn truncate(input: &str, max: usize) -> String {
    if input.len() <= max {
        input.to_string()
    } else {
        let mut out = input[..max].to_string();
        out.push_str("...");
        out
    }
}

pub(super) fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub(super) fn is_command_input(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with(':') || trimmed.starts_with('>')
}
