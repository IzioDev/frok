use crate::build_info;
use crate::connection::AgentSender;
use crate::ui::AppState;
use anyhow::{Context, Result, bail};
use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_dalek::{Signer, SigningKey};
use frok_common::paths::data_path;
use frok_common::time::now_ts;
use frok_common::wire::read_wire_message;
use frok_protocol::{AuthMethod, ClientCommands, OidcParams, ServerMessage, WireMessage};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, RedirectUrl, RefreshToken, Scope, TokenResponse,
};
use rand::rngs::OsRng;
use reqwest::redirect::Policy;
use std::env;
use std::fs;
use std::future::Future;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

pub struct AuthTracker {
    authenticated: AtomicBool,
    hello_sent: AtomicBool,
}

impl AuthTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            authenticated: AtomicBool::new(false),
            hello_sent: AtomicBool::new(false),
        })
    }

    pub fn mark_authenticated(&self) {
        self.authenticated.store(true, Ordering::Release);
    }

    pub fn mark_unauthenticated(&self) {
        self.authenticated.store(false, Ordering::Release);
    }

    pub fn mark_hello_sent(&self) {
        self.hello_sent.store(true, Ordering::Release);
    }

    pub fn reset_hello_sent(&self) {
        self.hello_sent.store(false, Ordering::Release);
    }

    pub fn hello_sent(&self) -> bool {
        self.hello_sent.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
pub struct AgentAuthConfig {
    pub agent_label: String,
    pub oidc_code: Option<String>,
    pub manual_tty: bool,
    pub manual_ui: bool,
}

type ManualInputFuture = Pin<Box<dyn Future<Output = Result<String>> + Send>>;

pub async fn perform_handshake(
    recv: &mut quinn::RecvStream,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    config: AgentAuthConfig,
    tracker: Arc<AuthTracker>,
) -> Result<()> {
    if skip_auth_handshake() {
        tracker.mark_authenticated();
        if !tracker.hello_sent() {
            sender
                .hello(config.agent_label, build_info::local_build_info())
                .await?;
            tracker.mark_hello_sent();
        }
        return Ok(());
    }

    let mut buffer = Vec::new();
    loop {
        let message = read_wire_message(recv, &mut buffer).await?;
        match message {
            WireMessage::Server(ServerMessage::AuthChallenge { nonce, oidc }) => {
                respond_to_auth_challenge(
                    sender.clone(),
                    state.clone(),
                    config.clone(),
                    nonce,
                    oidc,
                )
                .await?;
                wait_for_auth_ok(recv, sender, state, config, tracker).await?;
                break;
            }
            WireMessage::Server(ServerMessage::AuthOk {
                subject,
                session_id,
            }) => {
                state
                    .log_info(format!("auth ok: {subject} (session {session_id})"))
                    .await;
                tracker.mark_authenticated();
                if !tracker.hello_sent() {
                    sender
                        .hello(config.agent_label, build_info::local_build_info())
                        .await?;
                    tracker.mark_hello_sent();
                }
                break;
            }
            WireMessage::Server(ServerMessage::AuthErr { reason }) => {
                tracker.mark_unauthenticated();
                tracker.reset_hello_sent();
                bail!("auth failed: {reason}");
            }
            WireMessage::Server(ServerMessage::HelloAck { build, .. }) => {
                // Pre-auth hello ack can arrive before auth completes; capture build info quietly.
                state.set_edge_build(Some(build)).await;
            }
            other => {
                state
                    .log_warn(format!("unexpected pre-auth message: {other:?}"))
                    .await;
            }
        }
    }

    Ok(())
}

fn skip_auth_handshake() -> bool {
    matches!(
        env::var("FROK_INSECURE_SKIP_AUTH").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES") | Ok("on") | Ok("ON")
    )
}

async fn wait_for_auth_ok(
    recv: &mut quinn::RecvStream,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    config: AgentAuthConfig,
    tracker: Arc<AuthTracker>,
) -> Result<()> {
    let mut buffer = Vec::new();
    loop {
        let message = read_wire_message(recv, &mut buffer).await?;
        match message {
            WireMessage::Server(ServerMessage::AuthOk {
                subject,
                session_id,
            }) => {
                state
                    .log_info(format!("auth ok: {subject} (session {session_id})"))
                    .await;
                tracker.mark_authenticated();
                if !tracker.hello_sent() {
                    sender
                        .hello(config.agent_label, build_info::local_build_info())
                        .await?;
                    tracker.mark_hello_sent();
                }
                return Ok(());
            }
            WireMessage::Server(ServerMessage::AuthErr { reason }) => {
                bail!("auth failed: {reason}");
            }
            WireMessage::Server(ServerMessage::HelloAck { build, .. }) => {
                // Ignore pre-auth hello ack during handshake, but keep build info.
                state.set_edge_build(Some(build)).await;
            }
            other => {
                state
                    .log_warn(format!("unexpected auth message: {other:?}"))
                    .await;
            }
        }
    }
}

pub async fn handle_auth_challenge(
    nonce: Vec<u8>,
    oidc: Option<OidcParams>,
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    config: AgentAuthConfig,
) -> Result<()> {
    respond_to_auth_challenge(sender.clone(), state, config.clone(), nonce, oidc).await?;
    Ok(())
}

async fn respond_to_auth_challenge(
    sender: Arc<AgentSender>,
    state: Arc<AppState>,
    config: AgentAuthConfig,
    nonce: Vec<u8>,
    oidc: Option<OidcParams>,
) -> Result<()> {
    let method = if let Some(params) = oidc {
        let token = oidc_token(params, state.clone(), config.clone()).await?;
        AuthMethod::Oidc {
            token,
            agent_label: config.agent_label.clone(),
        }
    } else {
        let key = load_or_generate_key()?;
        let signature = key.sign(&nonce);
        AuthMethod::Key {
            public_key: key.verifying_key().to_bytes().to_vec(),
            signature: signature.to_bytes().to_vec(),
            agent_label: config.agent_label.clone(),
        }
    };

    sender.auth(method).await?;
    Ok(())
}

async fn oidc_token(
    params: OidcParams,
    state: Arc<AppState>,
    config: AgentAuthConfig,
) -> Result<String> {
    if let Some(token) = load_cached_token()? {
        if token.is_valid()? {
            return Ok(token.token);
        }
        if let Some(refresh) = token.refresh_token.clone() {
            if let Ok(updated) = refresh_access_token(&params, refresh).await {
                store_cached_token(&updated)?;
                return Ok(updated.token);
            }
        }
    }

    let token = interactive_login(&params, state, config).await?;
    store_cached_token(&token)?;
    Ok(token.token)
}

async fn interactive_login(
    params: &OidcParams,
    state: Arc<AppState>,
    config: AgentAuthConfig,
) -> Result<TokenCache> {
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .context("oidc http client")?;
    let issuer = IssuerUrl::new(params.issuer.clone()).context("invalid issuer url")?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &http)
        .await
        .context("oidc discovery")?;

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let client =
        CoreClient::from_provider_metadata(metadata, ClientId::new(params.audience.clone()), None)
            .set_redirect_uri(RedirectUrl::new(redirect_uri.clone())?);

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let expected_state = csrf_token.secret().to_string();

    state
        .set_auth_url(Some(auth_url.as_str().to_string()))
        .await;
    state.log_info(format!("OIDC login URL: {auth_url}")).await;
    state.log_info("opening browser for OIDC login").await;
    if let Err(err) = webbrowser::open(auth_url.as_str()) {
        state
            .log_warn(format!(
                "unable to open browser for OIDC login: {err} (open URL manually)"
            ))
            .await;
    }

    let (auth_code_tx, auth_code_rx) = mpsc::unbounded_channel::<String>();
    if config.manual_ui {
        state.set_auth_code_sender(Some(auth_code_tx)).await;
    } else {
        state.set_auth_code_sender(None).await;
    }

    let login_result = async {
        if let Some(mut manual) =
            build_manual_input_future(expected_state.clone(), state.clone(), config, auth_code_rx)
                .await?
        {
            let callback = await_callback(listener, port, &expected_state);
            tokio::select! {
                code = callback => code,
                code = &mut manual => code,
            }
        } else {
            await_callback(listener, port, &expected_state).await
        }
    }
    .await;

    state.set_auth_code_sender(None).await;
    state.set_auth_url(None).await;
    let code = login_result?;

    let token_response = client
        .exchange_code(AuthorizationCode::new(code))?
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http)
        .await
        .context("exchange code")?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| anyhow::anyhow!("missing id_token"))?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &nonce)
        .context("verify id_token")?;

    let expires_at = claims.expiration().timestamp() - 30;
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_string());
    let token = id_token.to_string();
    Ok(TokenCache {
        token,
        refresh_token,
        expires_at: Some(expires_at),
    })
}

async fn await_callback(listener: TcpListener, port: u16, expected_state: &str) -> Result<String> {
    let (mut stream, _) = listener.accept().await?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let line = request.lines().next().unwrap_or_default();
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let url = format!("http://127.0.0.1:{port}{path}");
    let parsed = url::Url::parse(&url)?;
    let code = extract_code_from_url(&parsed, expected_state)?;

    let body = "Authentication complete. You can close this window.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;

    Ok(code)
}

async fn build_manual_input_future(
    expected_state: String,
    state: Arc<AppState>,
    config: AgentAuthConfig,
    auth_code_rx: mpsc::UnboundedReceiver<String>,
) -> Result<Option<ManualInputFuture>> {
    if let Some(input) = config.oidc_code {
        return Ok(Some(Box::pin(async move {
            parse_manual_input(input, &expected_state, state.clone()).await
        })));
    }

    let allow_tty = config.manual_tty && std::io::stdin().is_terminal();
    let allow_ui = config.manual_ui;

    if !allow_tty && !allow_ui {
        return Ok(None);
    }

    Ok(Some(Box::pin(async move {
        let mut stdin_future: Option<ManualInputFuture> = None;
        if allow_tty {
            state
                .log_info("Enter code (optional; browser callback may complete automatically):")
                .await;
            stdin_future = Some(Box::pin(async move {
                let mut reader = BufReader::new(tokio::io::stdin());
                let mut input = String::new();
                reader.read_line(&mut input).await?;
                Ok(input)
            }));
        }

        let mut ui_future: Option<ManualInputFuture> = None;
        if allow_ui {
            let mut rx = auth_code_rx;
            ui_future = Some(Box::pin(async move {
                rx.recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("auth input channel closed"))
            }));
        }

        let input = match (stdin_future.as_mut(), ui_future.as_mut()) {
            (Some(stdin_future), Some(ui_future)) => {
                tokio::select! {
                    input = stdin_future => input?,
                    input = ui_future => input?,
                }
            }
            (Some(stdin_future), None) => stdin_future.await?,
            (None, Some(ui_future)) => ui_future.await?,
            (None, None) => bail!("manual input not available"),
        };

        parse_manual_input(input, &expected_state, state.clone()).await
    })))
}

async fn parse_manual_input(
    input: String,
    expected_state: &str,
    state: Arc<AppState>,
) -> Result<String> {
    let (code, verified) = extract_code_from_input(&input, expected_state)?;
    if !verified {
        state
            .log_warn("OIDC code provided without state; consider pasting the full URL")
            .await;
    }
    Ok(code)
}

fn extract_code_from_input(input: &str, expected_state: &str) -> Result<(String, bool)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("missing authorization code");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let parsed = url::Url::parse(trimmed)?;
        let code = extract_code_from_url(&parsed, expected_state)?;
        return Ok((code, true));
    }

    Ok((trimmed.to_string(), false))
}

fn extract_code_from_url(parsed: &url::Url, expected_state: &str) -> Result<String> {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            "error" => error = Some(value.to_string()),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Err(anyhow::anyhow!("oidc error: {error}"));
    }
    if state.as_deref() != Some(expected_state) {
        return Err(anyhow::anyhow!("oidc state mismatch"));
    }

    code.ok_or_else(|| anyhow::anyhow!("missing authorization code"))
}

async fn refresh_access_token(params: &OidcParams, refresh: String) -> Result<TokenCache> {
    let issuer = IssuerUrl::new(params.issuer.clone()).context("invalid issuer url")?;
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .context("oidc http client")?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &http).await?;
    let client =
        CoreClient::from_provider_metadata(metadata, ClientId::new(params.audience.clone()), None);
    let refresh_fallback = refresh.clone();
    let token_response = client
        .exchange_refresh_token(&RefreshToken::new(refresh))?
        .request_async(&http)
        .await?;

    let id_token = token_response
        .id_token()
        .ok_or_else(|| anyhow::anyhow!("missing id_token"))?;
    let claims = id_token
        .claims(&client.id_token_verifier(), ignore_nonce)
        .context("verify id_token")?;

    let expires_at = claims.expiration().timestamp() - 30;
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_string())
        .or(Some(refresh_fallback));
    let token = id_token.to_string();
    Ok(TokenCache {
        token,
        refresh_token,
        expires_at: Some(expires_at),
    })
}

fn ignore_nonce(_: Option<&Nonce>) -> Result<(), String> {
    Ok(())
}

fn load_or_generate_key() -> Result<SigningKey> {
    let path = key_path();
    if path.exists() {
        let bytes = fs::read(&path)?;
        let key_bytes: [u8; 32] = bytes.as_slice().try_into().context("invalid key length")?;
        return Ok(SigningKey::from_bytes(&key_bytes));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let key = SigningKey::generate(&mut OsRng);
    fs::write(&path, key.to_bytes())?;
    Ok(key)
}

fn key_path() -> PathBuf {
    data_path("agent-key.bin")
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
struct TokenCache {
    token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
}

impl TokenCache {
    fn is_valid(&self) -> Result<bool> {
        match self.expires_at {
            Some(ts) => Ok(now_ts()? < ts),
            None => Ok(false),
        }
    }
}

fn token_cache_path() -> PathBuf {
    data_path("oidc-cache.bin")
}

fn load_cached_token() -> Result<Option<TokenCache>> {
    let path = token_cache_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let token = TokenCache::try_from_slice(&bytes)?;
    Ok(Some(token))
}

fn store_cached_token(token: &TokenCache) -> Result<()> {
    let path = token_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = borsh::to_vec(token)?;
    fs::write(path, bytes)?;
    Ok(())
}
