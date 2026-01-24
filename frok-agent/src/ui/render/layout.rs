use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub(super) fn split_main(area: Rect) -> (Option<Rect>, Rect) {
    let height = area.height;
    if height == 0 {
        return (None, area);
    }

    let mut header = 3u16;
    if height < 10 {
        header = 2;
    }
    if height < 6 {
        header = 1;
    }
    if height < 4 {
        header = 0;
    }

    let body = height.saturating_sub(header);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header), Constraint::Length(body)])
        .split(area);

    let header_area = if header > 0 { Some(sections[0]) } else { None };
    (header_area, sections[1])
}

pub(super) fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(popup_layout[1])[1]
}
