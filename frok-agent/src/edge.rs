use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use quinn::TransportConfig;
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;

pub(crate) fn configure_client_transport(
    mut client: quinn::ClientConfig,
) -> Result<quinn::ClientConfig> {
    let mut transport = TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(10)));
    transport.max_idle_timeout(Some(Duration::from_secs(60).try_into()?));
    transport.initial_rtt(Duration::from_millis(10));
    transport.initial_mtu(1200);
    transport.min_mtu(1200);
    transport.mtu_discovery_config(None);
    transport.send_fairness(false);
    transport.stream_receive_window(quinn::VarInt::from_u32(16 * 1024 * 1024));
    transport.receive_window(quinn::VarInt::from_u32(64 * 1024 * 1024));
    transport.send_window(64 * 1024 * 1024);
    client.transport_config(Arc::new(transport));
    Ok(client)
}

pub(crate) fn parse_edge_target(input: &str) -> Result<(String, u16)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Edge endpoint is empty");
    }

    let without_scheme = if let Some((_, rest)) = trimmed.split_once("://") {
        rest
    } else {
        trimmed
    };

    if without_scheme.starts_with('[') {
        let Some(end) = without_scheme.find(']') else {
            bail!("Invalid IPv6 address format: {trimmed}");
        };
        let host = &without_scheme[1..end];
        let remainder = &without_scheme[end + 1..];
        let port = if remainder.starts_with(':') {
            parse_port(&remainder[1..])?
        } else if remainder.is_empty() {
            443
        } else {
            bail!("Invalid edge endpoint: {trimmed}");
        };
        return Ok((host.to_string(), port));
    }

    let colon_count = without_scheme.chars().filter(|c| *c == ':').count();
    if colon_count > 1 {
        bail!("IPv6 addresses must be wrapped in brackets: [::1]:443");
    }

    if let Some((host, port)) = without_scheme.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return Ok((host.to_string(), parse_port(port)?));
        }
    }

    Ok((without_scheme.to_string(), 443))
}

fn parse_port(port: &str) -> Result<u16> {
    port.parse::<u16>()
        .with_context(|| format!("Invalid port: {port}"))
}

pub(crate) async fn resolve_edge_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolve edge host {host}:{port}"))?;
    let mut addrs = addrs.collect::<Vec<_>>();
    if addrs.is_empty() {
        bail!("No addresses found for {host}:{port}");
    }

    addrs.sort_by_key(|addr| match addr {
        SocketAddr::V6(_) => 0,
        SocketAddr::V4(_) => 1,
    });
    addrs.dedup();
    Ok(addrs)
}

pub(crate) fn build_client_config() -> Result<quinn::ClientConfig> {
    let mut roots = RootCertStore::empty();
    let native_certs = rustls_native_certs::load_native_certs().context("load native certs")?;
    for cert in native_certs {
        roots.add(cert).context("add native cert")?;
    }
    if dev_tls_enabled() {
        add_dev_ca(&mut roots).context("load dev tls ca")?;
    }
    let client = quinn::ClientConfig::with_root_certificates(Arc::new(roots))
        .context("client tls config")?;
    Ok(client)
}

fn dev_tls_enabled() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    matches!(
        std::env::var("FROK_DEV_TLS").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES") | Ok("on") | Ok("ON")
    )
}

fn add_dev_ca(roots: &mut RootCertStore) -> Result<()> {
    let _ = dev_tls::load_or_generate()?;
    let ca_path = PathBuf::from("target")
        .join("quinn-dev-certs")
        .join("ca.der");
    let data = fs::read(&ca_path)
        .with_context(|| format!("read dev tls ca from {}", ca_path.display()))?;
    roots
        .add(CertificateDer::from(data))
        .context("add dev tls ca")?;
    Ok(())
}
