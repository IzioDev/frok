use std::borrow::Cow;

use anyhow::{Result, bail};

use frok_common::normalize_host;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedHost {
    pub(crate) name: String,
    pub(crate) full_host: String,
    pub(crate) domain_mismatch: bool,
}

pub(crate) fn normalize_route_name(input: &str) -> Cow<'_, str> {
    let normalized = normalize_host(input);
    match normalized.split_once('.') {
        Some((name, _)) => Cow::Owned(name.to_string()),
        None => normalized,
    }
}

pub(crate) fn resolve_public_host(input: &str, public_domain: &str) -> Result<ResolvedHost> {
    let normalized = normalize_host(input);
    let normalized = normalized.as_ref();
    if normalized.is_empty() {
        bail!("route name is empty");
    }

    let (name_part, domain_part) = match normalized.split_once('.') {
        Some((name, rest)) => (name, Some(rest)),
        None => (normalized, None),
    };

    validate_dns_label(name_part)?;

    let public_domain = normalize_host(public_domain);
    let public_domain = public_domain.as_ref();
    if public_domain.is_empty() {
        bail!("public domain is empty");
    }

    let domain_mismatch = domain_part
        .map(|domain| normalize_host(domain).into_owned())
        .as_deref()
        .map(|domain| domain != public_domain)
        .unwrap_or(false);

    let name = name_part.to_string();
    let full_host = format!("{name}.{public_domain}");

    Ok(ResolvedHost {
        name,
        full_host,
        domain_mismatch,
    })
}

fn validate_dns_label(label: &str) -> Result<()> {
    if label.is_empty() {
        bail!("route name is empty");
    }
    if label.len() > 63 {
        bail!("route name is too long (max 63 characters)");
    }

    let bytes = label.as_bytes();
    let is_alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();

    if !is_alnum(bytes[0]) || !is_alnum(bytes[bytes.len() - 1]) {
        bail!("route name must start and end with a letter or digit");
    }

    for &b in bytes {
        if !(is_alnum(b) || b == b'-') {
            bail!("route name may only contain letters, digits, or hyphens");
        }
    }

    Ok(())
}
