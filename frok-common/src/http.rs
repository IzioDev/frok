use std::borrow::Cow;

use frok_protocol::{ByteBuf, Header};

use crate::host::normalize_host;

pub fn is_hop_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-connection")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
}

pub fn header_bytes_eq(bytes: &ByteBuf, expected: &str) -> bool {
    std::str::from_utf8(bytes.as_bytes())
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

pub fn is_hop_header_bytes(name: &ByteBuf) -> bool {
    std::str::from_utf8(name.as_bytes())
        .map(is_hop_header)
        .unwrap_or(false)
}

pub fn extract_host(headers: &[Header]) -> Option<Cow<'_, str>> {
    for header in headers {
        let name = header.name_str()?;
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case(":authority") {
            let value = header.value_str()?;
            let host = value.split(':').next().unwrap_or(value);
            return Some(normalize_host(host));
        }
    }
    None
}
