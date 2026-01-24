use anyhow::{Context, Result};
use quinn::{ClientConfig, ServerConfig};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::{fs, net::IpAddr, path::PathBuf, sync::Arc};

pub struct DevTls {
    pub server: ServerConfig,
    pub client: ClientConfig,
}

pub fn load_or_generate() -> Result<DevTls> {
    let dir = dev_dir();
    fs::create_dir_all(&dir).context("create dev cert dir")?;

    let ca_path = dir.join("ca.der");
    let server_cert_path = dir.join("server.der");
    let server_key_path = dir.join("server-key.pkcs8.der");

    let (ca_der, server_der, server_key_der) =
        if ca_path.exists() && server_cert_path.exists() && server_key_path.exists() {
            (
                fs::read(&ca_path).context("read ca.der")?,
                fs::read(&server_cert_path).context("read server.der")?,
                fs::read(&server_key_path).context("read server key")?,
            )
        } else {
            let g = generate()?;
            fs::write(&ca_path, &g.ca_der).context("write ca.der")?;
            fs::write(&server_cert_path, &g.server_der).context("write server.der")?;
            fs::write(&server_key_path, &g.server_key_pkcs8_der).context("write server key")?;
            (g.ca_der, g.server_der, g.server_key_pkcs8_der)
        };

    // Server config (cert chain + key)
    let cert_chain = vec![CertificateDer::from(server_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key_der));
    let server = ServerConfig::with_single_cert(cert_chain, key).context("server tls config")?;

    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(ca_der))
        .context("add dev CA to root store")?;
    let client =
        ClientConfig::with_root_certificates(Arc::new(roots)).context("client tls config")?;

    Ok(DevTls { server, client })
}

struct Generated {
    ca_der: Vec<u8>,
    server_der: Vec<u8>,
    server_key_pkcs8_der: Vec<u8>,
}

fn generate() -> Result<Generated> {
    let ca_key = KeyPair::generate().context("generate CA key")?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).context("CA params")?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let mut ca_dn = DistinguishedName::new();
    ca_dn.push(DnType::CommonName, "quinn-dev-ca");
    ca_params.distinguished_name = ca_dn;

    let ca_cert = ca_params.self_signed(&ca_key).context("self-sign CA")?;
    let ca_der = ca_cert.der().as_ref().to_vec();

    let issuer = Issuer::new(ca_params, ca_key);

    let server_key = KeyPair::generate().context("generate server key")?;
    let mut server_params =
        CertificateParams::new(vec!["localhost".to_string()]).context("server params")?;
    // Include IP SANs or rustls/clients will reject for 127.0.0.1
    server_params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::from([127, 0, 0, 1])));
    server_params
        .subject_alt_names
        .push(SanType::IpAddress("::1".parse().unwrap()));
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];

    let server_cert = server_params
        .signed_by(&server_key, &issuer)
        .context("sign server cert")?;
    let server_der = server_cert.der().as_ref().to_vec();
    let server_key_pkcs8_der = server_key.serialize_der();

    Ok(Generated {
        ca_der,
        server_der,
        server_key_pkcs8_der,
    })
}

fn dev_dir() -> PathBuf {
    PathBuf::from("target").join("quinn-dev-certs")
}
