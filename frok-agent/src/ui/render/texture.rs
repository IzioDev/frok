use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use super::super::theme::{Texture, Theme};

pub(super) fn render_texture(frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
    match theme.texture {
        Texture::None => {}
        Texture::Scanlines => render_scanlines(frame, area, theme),
        Texture::Grid => render_grid(frame, area, theme),
    }
}

fn render_scanlines(frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let width = area.width as usize;
    let line = " ".repeat(width);
    let mut lines = Vec::with_capacity(area.height as usize);
    for row in 0..area.height {
        let color = if row % 2 == 0 {
            theme.texture_color
        } else {
            theme.bg
        };
        lines.push(Line::from(Span::styled(
            line.as_str(),
            Style::default().bg(color),
        )));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_grid(frame: &mut ratatui::Frame, area: Rect, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let mut lines = Vec::with_capacity(area.height as usize);
    let width = area.width as usize;
    for row in 0..area.height as usize {
        let mut line = String::with_capacity(width);
        for col in 0..width {
            let ch = if row % 4 == 0 {
                if col % 8 == 0 { '+' } else { '-' }
            } else if col % 8 == 0 {
                '|'
            } else {
                ' '
            };
            line.push(ch);
        }
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(theme.texture_color),
        )));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}
