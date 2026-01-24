use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::super::input::slice_visible;
use super::super::theme::Theme;

pub(super) fn render_text_input(
    frame: &mut ratatui::Frame,
    area: Rect,
    label: &str,
    input: &super::super::app_state::TextInput,
    theme: &Theme,
    placeholder: Option<&str>,
) -> Option<(u16, u16)> {
    if area.height == 0 || area.width == 0 {
        return None;
    }

    let label = format!("{label}: ");
    let label_len = label.len();
    let width = area.width.saturating_sub(label_len as u16) as usize;
    let (visible, cursor_visible, offset) = slice_visible(&input.value, input.cursor, width);

    let mut spans = vec![Span::styled(
        label.as_str(),
        Style::default().fg(theme.muted),
    )];

    if visible.is_empty() {
        if let Some(placeholder) = placeholder {
            spans.push(Span::styled(placeholder, Style::default().fg(theme.muted)));
        }
    } else {
        spans.push(Span::styled(visible, Style::default().fg(theme.text)));
    }

    if offset > 0 {
        spans.insert(1, Span::styled("<", Style::default().fg(theme.muted)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    let cursor_x = area.x + label_len as u16 + cursor_visible as u16;
    let cursor_y = area.y;
    Some((cursor_x, cursor_y))
}
