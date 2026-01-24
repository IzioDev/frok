use std::borrow::Cow;

pub fn normalize_host(host: &str) -> Cow<'_, str> {
    let trimmed = host.trim().trim_end_matches('.');
    if trimmed.as_bytes().iter().all(|b| !b.is_ascii_uppercase()) {
        Cow::Borrowed(trimmed)
    } else {
        Cow::Owned(trimmed.to_ascii_lowercase())
    }
}
