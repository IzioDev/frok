use crate::store::{AuthKind, EdgeStore, KeyTrustRecord};
use anyhow::{Context, Result, anyhow, bail};
use dashmap::DashMap;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use frok_common::time::now_ts;
use frok_protocol::{AuthMethod, OidcParams};
use openidconnect::core::{CoreClient, CoreIdToken, CoreJwsSigningAlgorithm, CoreProviderMetadata};
use openidconnect::{ClientId, IssuerUrl};
use rand::RngCore;
use rand::rngs::OsRng;
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct AuthResult {
    pub subject: String,
    pub auth_kind: AuthKind,
    pub agent_label: String,
    pub public_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AuthMode {
    Insecure,
    Oidc(OidcConfig),
    Key(KeyConfig),
}

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: IssuerUrl,
    pub audience: ClientId,
    pub allowed_algs: Vec<CoreJwsSigningAlgorithm>,
    pub allowed_alg_names: Vec<String>,
    jwks_cache_ttl: Duration,
    cache: Arc<RwLock<OidcCache>>,
    http: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct KeyConfig {
    pub tofu: bool,
}

#[derive(Debug, Clone)]
struct OidcCache {
    metadata: CoreProviderMetadata,
    fetched_at: Instant,
}

pub struct EdgeAuth {
    mode: AuthMode,
    nonce_ttl: Duration,
    used_nonces: DashMap<Vec<u8>, Instant>,
}

impl EdgeAuth {
    pub fn new(mode: AuthMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            nonce_ttl: Duration::from_secs(60),
            used_nonces: DashMap::new(),
        })
    }

    pub fn is_insecure(&self) -> bool {
        matches!(self.mode, AuthMode::Insecure)
    }

    pub fn mode(&self) -> &AuthMode {
        &self.mode
    }

    pub fn make_challenge(&self) -> Option<(Vec<u8>, Option<OidcParams>)> {
        match &self.mode {
            AuthMode::Insecure => None,
            AuthMode::Oidc(config) => {
                let nonce = random_nonce();
                let params = OidcParams {
                    issuer: config.issuer.as_str().to_string(),
                    audience: config.audience.as_str().to_string(),
                    allowed_algs: config.allowed_alg_names.clone(),
                };
                Some((nonce, Some(params)))
            }
            AuthMode::Key(_) => {
                let nonce = random_nonce();
                Some((nonce, None))
            }
        }
    }

    pub async fn verify(
        &self,
        store: &EdgeStore,
        method: AuthMethod,
        expected_nonce: &[u8],
        issued_at: Instant,
    ) -> Result<AuthResult> {
        if issued_at.elapsed() > self.nonce_ttl {
            bail!("auth challenge expired");
        }

        if self.used_nonces.contains_key(expected_nonce) {
            bail!("auth nonce replayed");
        }

        let result = match (&self.mode, method) {
            (AuthMode::Oidc(config), AuthMethod::Oidc { token, agent_label }) => {
                let bearer = token.trim();
                let bearer = bearer.strip_prefix("Bearer ").unwrap_or(bearer);
                let metadata = config.metadata().await?;
                let client =
                    CoreClient::from_provider_metadata(metadata, config.audience.clone(), None);
                let mut verifier = client.id_token_verifier();
                if !config.allowed_algs.is_empty() {
                    verifier = verifier.set_allowed_algs(config.allowed_algs.clone());
                }
                let id_token = CoreIdToken::from_str(bearer).context("invalid id token")?;
                let claims = id_token
                    .claims(&verifier, ignore_nonce)
                    .context("oidc token verify")?;
                let subject = claims.subject().as_str().to_string();
                if subject.is_empty() {
                    bail!("oidc token missing subject");
                }
                Ok(AuthResult {
                    subject,
                    auth_kind: AuthKind::Oidc,
                    agent_label,
                    public_key_fingerprint: None,
                })
            }
            (
                AuthMode::Key(config),
                AuthMethod::Key {
                    public_key,
                    signature,
                    agent_label,
                },
            ) => {
                if public_key.len() != 32 {
                    bail!("invalid key length");
                }
                if signature.len() != 64 {
                    bail!("invalid signature length");
                }
                let public_key = VerifyingKey::from_bytes(public_key.as_slice().try_into()?)
                    .context("invalid public key")?;
                let signature = Signature::from_slice(&signature).context("invalid signature")?;
                public_key
                    .verify(expected_nonce, &signature)
                    .context("invalid signature")?;

                let fingerprint = fingerprint_key(&public_key.to_bytes());
                if let Some(record) = store.load_key_trust(&fingerprint)? {
                    let now = now_ts()?;
                    let updated = KeyTrustRecord {
                        public_key_fingerprint: fingerprint.clone(),
                        first_seen_at: record.first_seen_at,
                        last_seen_at: now,
                        agent_label: agent_label.clone(),
                    };
                    store.upsert_key_trust(&updated)?;
                } else if config.tofu {
                    let now = now_ts()?;
                    let record = KeyTrustRecord {
                        public_key_fingerprint: fingerprint.clone(),
                        first_seen_at: now,
                        last_seen_at: now,
                        agent_label: agent_label.clone(),
                    };
                    store.upsert_key_trust(&record)?;
                } else {
                    bail!("untrusted key");
                }

                Ok(AuthResult {
                    subject: fingerprint.clone(),
                    auth_kind: AuthKind::Key,
                    agent_label,
                    public_key_fingerprint: Some(fingerprint),
                })
            }
            (AuthMode::Insecure, _) => Err(anyhow!("auth not required")),
            (AuthMode::Oidc(_), _) => Err(anyhow!("oidc auth required")),
            (AuthMode::Key(_), _) => Err(anyhow!("key auth required")),
        }?;

        self.used_nonces
            .insert(expected_nonce.to_vec(), Instant::now());
        Ok(result)
    }
}

pub async fn build_oidc_verifier(
    issuer: IssuerUrl,
    audience: ClientId,
    allowed_algs: Vec<CoreJwsSigningAlgorithm>,
    allowed_alg_names: Vec<String>,
    jwks_cache_ttl: Duration,
) -> Result<OidcConfig> {
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .context("oidc http client")?;

    let metadata = CoreProviderMetadata::discover_async(issuer.clone(), &http).await?;
    let cache = OidcCache {
        metadata,
        fetched_at: Instant::now(),
    };

    Ok(OidcConfig {
        issuer,
        audience,
        allowed_algs,
        allowed_alg_names,
        jwks_cache_ttl,
        cache: Arc::new(RwLock::new(cache)),
        http,
    })
}

fn random_nonce() -> Vec<u8> {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    buf.to_vec()
}

fn fingerprint_key(bytes: &[u8; 32]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn ignore_nonce(_: Option<&openidconnect::Nonce>) -> Result<(), String> {
    Ok(())
}

impl OidcConfig {
    async fn metadata(&self) -> Result<CoreProviderMetadata> {
        let now = Instant::now();
        {
            let guard = self.cache.read().await;
            if now.duration_since(guard.fetched_at) <= self.jwks_cache_ttl {
                return Ok(guard.metadata.clone());
            }
        }

        let metadata =
            CoreProviderMetadata::discover_async(self.issuer.clone(), &self.http).await?;
        let mut guard = self.cache.write().await;
        guard.metadata = metadata.clone();
        guard.fetched_at = Instant::now();
        Ok(metadata)
    }
}
