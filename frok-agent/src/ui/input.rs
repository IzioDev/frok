pub(crate) fn slice_visible(input: &str, cursor: usize, width: usize) -> (String, usize, usize) {
    if width == 0 {
        return (String::new(), 0, 0);
    }
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);

    if len <= width {
        let visible = chars.iter().collect::<String>();
        return (visible, cursor, 0);
    }

    let mut start = 0;
    if cursor > width {
        start = cursor - width + 1;
    }
    let max_start = len - width;
    if start > max_start {
        start = max_start;
    }
    let end = (start + width).min(len);
    let visible = chars[start..end].iter().collect();
    let cursor_visible = cursor.saturating_sub(start).min(width);
    (visible, cursor_visible, start)
}

pub(crate) fn fuzzy_match(query: &str, candidate: &str) -> bool {
    let mut q = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase());
    let mut current = q.next();
    if current.is_none() {
        return true;
    }

    for ch in candidate.chars() {
        if let Some(target) = current {
            if ch.to_ascii_lowercase() == target {
                current = q.next();
                if current.is_none() {
                    return true;
                }
            }
        } else {
            return true;
        }
    }

    current.is_none()
}
