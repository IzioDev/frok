use std::borrow::Cow;
use std::collections::VecDeque;

use super::app_state::LogsFilter;

pub(crate) fn is_error_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("failed")
        || lower.contains("panic")
        || lower.contains("disconnected")
}

pub(crate) fn matches_filter(line: &str, filter: LogsFilter) -> bool {
    match filter {
        LogsFilter::All => true,
        LogsFilter::Errors => is_error_line(line),
        LogsFilter::Events => !is_error_line(line),
    }
}

pub(crate) fn filter_logs<'a>(
    logs: &'a VecDeque<Cow<'static, str>>,
    filter: LogsFilter,
    search: &str,
) -> Vec<&'a Cow<'static, str>> {
    let query = search.trim().to_ascii_lowercase();
    logs.iter()
        .filter(|line| matches_filter(line, filter))
        .filter(|line| {
            if query.is_empty() {
                true
            } else {
                line.to_ascii_lowercase().contains(&query)
            }
        })
        .collect()
}
