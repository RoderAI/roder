use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::RngExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ISSUER: &str = "https://auth.x.ai";
const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const CALLBACK_PORT: u16 = 56121;
const CALLBACK_HOST: &str = "127.0.0.1";
const CALLBACK_PATH: &str = "/callback";
const REFRESH_EXPIRY_SKEW_MILLIS: i64 = 3 * 60 * 1000;

fn auth_client() -> Client {
    Client::builder()
        .user_agent("Roder/1.0 (+https://roder.sh)")
        .build()
        .expect("failed to construct reqwest client for xAI SuperGrok auth")
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    #[serde(rename = "type", default = "default_token_type")]
    pub token_type: String,
    #[serde(default)]
    pub refresh: String,
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub expires: i64,
    #[serde(default)]
    pub id_token: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    device_authorization_endpoint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    token_type: String,
}

#[derive(Debug, Default, Deserialize)]
struct DeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Default, Deserialize)]
struct DeviceTokenErrorResponse {
    error: String,
    error_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Claims {
    #[serde(default)]
    email: String,
}

pub struct Store {
    data_dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Self {
            data_dir: roder_data_dir(),
        }
    }

    pub fn load(&self) -> anyhow::Result<Tokens> {
        load_tokens_from(&self.path())
    }

    pub fn save(&self, mut tokens: Tokens) -> anyhow::Result<()> {
        normalize(&mut tokens);
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(&tokens)?;
        fs::write(path, [data, b"\n".to_vec()].concat())?;
        Ok(())
    }

    pub fn delete(&self) -> anyhow::Result<()> {
        let path = self.path();
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn path(&self) -> PathBuf {
        self.data_dir.join("auth").join("supergrok.json")
    }
}

pub async fn access_token() -> anyhow::Result<Option<String>> {
    let store = Store::new();
    let tokens = store.load()?;
    if tokens.refresh.trim().is_empty() && tokens.access.trim().is_empty() {
        return Ok(None);
    }
    let now = now_millis();
    if !tokens.access.trim().is_empty() && tokens.expires > now + REFRESH_EXPIRY_SKEW_MILLIS {
        return Ok(Some(tokens.access));
    }
    if tokens.refresh.trim().is_empty() {
        return Ok(None);
    }
    let mut refreshed = refresh(&tokens.refresh).await?;
    if refreshed.refresh.is_empty() {
        refreshed.refresh = tokens.refresh;
    }
    if refreshed.email.is_empty() {
        refreshed.email = tokens.email;
    }
    let access = refreshed.access.clone();
    store.save(refreshed)?;
    Ok(Some(access))
}

pub async fn login() -> anyhow::Result<Tokens> {
    let discovery = discover().await?;
    let pkce_verifier = random_string(64);
    let pkce_challenge = code_challenge(&pkce_verifier);
    let state = random_string(43);
    let nonce = random_string(43);
    let listener = match TcpListener::bind((CALLBACK_HOST, CALLBACK_PORT)) {
        Ok(l) => l,
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
            anyhow::bail!(
                "Port {CALLBACK_PORT} is already in use by another process.\n\
                 This usually happens if another roder instance, background app-server, or a previous login process is still running.\n\n\
                 Please close or kill that process to free the port. You can run the following command to force-kill it:\n\
                 lsof -t -i :{CALLBACK_PORT} | xargs kill -9\n"
            );
        }
        Err(err) => return Err(err.into()),
    };
    let redirect_uri = format!("http://{CALLBACK_HOST}:{CALLBACK_PORT}{CALLBACK_PATH}");
    let auth_url = authorize_url(
        &discovery.authorization_endpoint,
        &redirect_uri,
        &pkce_challenge,
        &state,
        &nonce,
    );

    open_browser(&auth_url).or_else(|err| {
        eprintln!("Could not open browser automatically: {err}");
        eprintln!("Open this SuperGrok sign-in URL manually: {auth_url}");
        Ok::<(), anyhow::Error>(())
    })?;

    let (code, returned_state) = wait_for_callback(listener)?;
    if returned_state != state {
        anyhow::bail!("invalid supergrok oauth state");
    }
    let tokens = exchange_code(
        &discovery.token_endpoint,
        &code,
        &redirect_uri,
        &pkce_verifier,
    )
    .await?;
    Store::new().save(tokens.clone())?;
    Ok(tokens)
}

pub async fn device_flow() -> anyhow::Result<(Tokens, Option<String>)> {
    let discovery = discover().await?;
    let device_endpoint = discovery
        .device_authorization_endpoint
        .clone()
        .ok_or_else(|| {
            anyhow::anyhow!("supergrok device authorization endpoint not available in discovery")
        })?;
    validate_xai_https_endpoint(&device_endpoint)?;

    let client = auth_client();
    let params = [("client_id", CLIENT_ID), ("scope", SCOPE)];
    let response = client.post(&device_endpoint).form(&params).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "supergrok device authorization request failed: {status} {}",
            text.trim()
        );
    }
    let device_auth: DeviceAuthorizationResponse = serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!(
            "supergrok device authorization response was not valid JSON: {e}
{text}"
        )
    })?;

    let verification_uri = device_auth
        .verification_uri_complete
        .as_ref()
        .unwrap_or(&device_auth.verification_uri);
    eprintln!("SuperGrok device sign-in");
    eprintln!("User code: {}", device_auth.user_code);
    eprintln!("Open: {verification_uri}");
    open_browser(verification_uri).or_else(|err| {
        eprintln!("Could not open browser automatically: {err}");
        Ok::<(), anyhow::Error>(())
    })?;

    let token = poll_device_token(
        &discovery.token_endpoint,
        &device_auth.device_code,
        device_auth.interval,
        device_auth.expires_in,
    )
    .await?;
    Store::new().save(token.clone())?;
    let email = if token.email.trim().is_empty() {
        None
    } else {
        Some(token.email.clone())
    };
    Ok((token, email))
}

async fn poll_device_token(
    token_endpoint: &str,
    device_code: &str,
    mut interval: u64,
    expires_in: u64,
) -> anyhow::Result<Tokens> {
    let max_interval = 60;
    if interval == 0 {
        interval = 5;
    }
    let expires_at = Instant::now() + Duration::from_secs(expires_in);
    let client = auth_client();
    loop {
        if Instant::now() >= expires_at {
            anyhow::bail!("supergrok device sign-in expired");
        }
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", CLIENT_ID),
        ];
        let response = client.post(token_endpoint).form(&params).send().await?;
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            let token_response = parse_token_response(&text)?;
            return Ok(tokens_from_response(token_response)?);
        }
        let error: DeviceTokenErrorResponse = match serde_json::from_str(&text) {
            Ok(e) => e,
            Err(_) => {
                anyhow::bail!(
                    "supergrok device token request failed: {status} {}",
                    text.trim()
                );
            }
        };
        match error.error.as_str() {
            "authorization_pending" => {
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
            "slow_down" => {
                interval = (interval + 5).min(max_interval);
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
            "expired_token" => {
                anyhow::bail!("supergrok device sign-in expired (expired_token)");
            }
            other => {
                let desc = error.error_description.as_deref().unwrap_or(other);
                anyhow::bail!("supergrok device sign-in error: {desc}");
            }
        }
    }
}

pub async fn status() -> anyhow::Result<Option<Tokens>> {
    let tokens = Store::new().load()?;
    Ok((!tokens.refresh.trim().is_empty()).then_some(tokens))
}

pub fn logout() -> anyhow::Result<()> {
    Store::new().delete()
}

async fn discover() -> anyhow::Result<DiscoveryDocument> {
    let response = auth_client().get(DISCOVERY_URL).send().await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("supergrok discovery failed: {status} {}", text.trim());
    }
    let discovery: DiscoveryDocument = serde_json::from_str(&text)?;
    validate_discovery(&discovery)?;
    Ok(discovery)
}

fn validate_discovery(discovery: &DiscoveryDocument) -> anyhow::Result<()> {
    if discovery.issuer != ISSUER {
        anyhow::bail!("supergrok discovery issuer mismatch");
    }
    validate_xai_https_endpoint(&discovery.authorization_endpoint)?;
    validate_xai_https_endpoint(&discovery.token_endpoint)?;
    if let Some(ref device_ep) = discovery.device_authorization_endpoint {
        validate_xai_https_endpoint(device_ep)?;
    }
    Ok(())
}

fn validate_xai_https_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let url = Url::parse(endpoint)?;
    if url.scheme() != "https" {
        anyhow::bail!("supergrok oauth endpoint must use https");
    }
    let host = url.host_str().unwrap_or_default();
    if host == "x.ai" || host.ends_with(".x.ai") {
        return Ok(());
    }
    anyhow::bail!("supergrok oauth endpoint must be hosted on x.ai");
}

fn load_tokens_from(path: &PathBuf) -> anyhow::Result<Tokens> {
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => Ok(Tokens::default()),
        Ok(contents) => {
            let mut tokens: Tokens = serde_json::from_str(&contents)?;
            normalize(&mut tokens);
            Ok(tokens)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Tokens::default()),
        Err(err) => Err(err.into()),
    }
}

async fn refresh(refresh_token: &str) -> anyhow::Result<Tokens> {
    let discovery = discover().await?;
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    token_request(&discovery.token_endpoint, &params).await
}

async fn exchange_code(
    token_endpoint: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> anyhow::Result<Tokens> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    token_request(token_endpoint, &params).await
}

async fn token_request(token_endpoint: &str, params: &[(&str, &str)]) -> anyhow::Result<Tokens> {
    validate_xai_https_endpoint(token_endpoint)?;
    let response = auth_client()
        .post(token_endpoint)
        .form(params)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "supergrok token request failed: {status} {}",
            redacted_body_excerpt(&text)
        );
    }
    let token_response = parse_token_response(&text)?;
    tokens_from_response(token_response)
}

fn parse_token_response(text: &str) -> anyhow::Result<TokenResponse> {
    serde_json::from_str(text).map_err(|err| {
        anyhow::anyhow!(
            "supergrok token response was not valid JSON: {err}; body: {}",
            redacted_body_excerpt(text)
        )
    })
}

fn tokens_from_response(response: TokenResponse) -> anyhow::Result<Tokens> {
    if response.access_token.trim().is_empty() {
        anyhow::bail!("supergrok token response missing access_token");
    }
    let expires_in = if response.expires_in > 0 {
        response.expires_in
    } else {
        3600
    };
    let mut tokens = Tokens {
        token_type: if response.token_type.is_empty() {
            default_token_type()
        } else {
            response.token_type
        },
        refresh: response.refresh_token,
        access: response.access_token,
        expires: now_millis() + expires_in * 1000,
        email: parse_email_claim(&response.id_token).unwrap_or_default(),
        id_token: response.id_token,
    };
    normalize(&mut tokens);
    Ok(tokens)
}

fn wait_for_callback(listener: TcpListener) -> anyhow::Result<(String, String)> {
    let (mut stream, _) = listener.accept()?;
    let mut buffer = [0_u8; 4096];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    let query = target.split_once('?').map(|(_, query)| query).unwrap_or("");
    let code = query_param(query, "code").unwrap_or_default();
    let state = query_param(query, "state").unwrap_or_default();
    if code.is_empty() {
        send_callback_html(&mut stream, false)?;
        anyhow::bail!("missing authorization code");
    }
    send_callback_html(&mut stream, true)?;
    Ok((code, state))
}

fn send_callback_html(stream: &mut TcpStream, ok: bool) -> anyhow::Result<()> {
    let body = if ok {
        "Connected to SuperGrok. You can close this tab."
    } else {
        "SuperGrok sign-in failed."
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn authorize_url(
    authorization_endpoint: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    nonce: &str,
) -> String {
    format!(
        "{authorization_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&nonce={}&plan=premium&referrer=roder",
        urlencoding::encode(CLIENT_ID),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(SCOPE),
        urlencoding::encode(challenge),
        urlencoding::encode(state),
        urlencoding::encode(nonce),
    )
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return urlencoding::decode(v).ok().map(|v| v.into_owned());
        }
    }
    None
}

fn parse_email_claim(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: Claims = serde_json::from_slice(&bytes).ok()?;
    (!claims.email.trim().is_empty()).then_some(claims.email)
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn random_string(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

fn open_browser(url: &str) -> anyhow::Result<()> {
    let mut command = browser_command(url);
    let status = command.status()?;
    if !status.success() {
        anyhow::bail!("failed to open browser");
    }
    Ok(())
}

fn browser_command(url: &str) -> std::process::Command {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    command.arg(url);
    command
}

fn redacted_body_excerpt(body: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 1_000;
    let mut excerpt = body.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    if body.chars().count() > MAX_ERROR_BODY_CHARS {
        excerpt.push_str(" ...");
    }
    for field in [
        "access_token",
        "refresh_token",
        "id_token",
        "access",
        "refresh",
    ] {
        redact_json_string_field(&mut excerpt, field);
    }
    excerpt
}

fn redact_json_string_field(body: &mut String, field: &str) {
    let pattern = format!("\"{field}\"");
    let mut search_from = 0;
    while let Some(relative_key_start) = body[search_from..].find(&pattern) {
        let key_start = search_from + relative_key_start;
        let Some(relative_colon) = body[key_start + pattern.len()..].find(':') else {
            return;
        };
        let value_scan_start = key_start + pattern.len() + relative_colon + 1;
        let Some(relative_quote) = body[value_scan_start..].find('"') else {
            search_from = value_scan_start;
            continue;
        };
        let value_start = value_scan_start + relative_quote;
        let mut escaped = false;
        let mut value_end = None;
        for (offset, ch) in body[value_start + 1..].char_indices() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                value_end = Some(value_start + 1 + offset);
                break;
            }
        }
        let Some(value_end) = value_end else {
            return;
        };
        body.replace_range(value_start + 1..value_end, "[redacted]");
        search_from = value_start + "\"[redacted]\"".len();
    }
}

fn roder_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".roder")
        })
}

fn normalize(tokens: &mut Tokens) {
    if tokens.token_type.is_empty() {
        tokens.token_type = default_token_type();
    }
    tokens.refresh = tokens.refresh.trim().to_string();
    tokens.access = tokens.access.trim().to_string();
    tokens.email = tokens.email.trim().to_string();
}

fn default_token_type() -> String {
    "oauth".to_string()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_url_matches_supergrok_oauth_shape() {
        let url = authorize_url(
            "https://auth.x.ai/oauth/authorize",
            "http://127.0.0.1:56121/callback",
            "challenge",
            "state",
            "nonce",
        );

        assert!(url.contains("client_id=b1a00492-073a-47ea-816f-4c329264a828"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access"));
        assert!(url.contains("grok-cli%3Aaccess"));
        assert!(url.contains("api%3Aaccess"));
        assert!(url.contains("plan=premium"));
        assert!(url.contains("referrer=roder"));
        assert!(url.contains("nonce=nonce"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    #[cfg(windows)]
    fn windows_browser_command_does_not_shell_split_oauth_url() {
        let url = "https://auth.x.ai/oauth/authorize?response_type=code&client_id=app";
        let command = browser_command(url);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program().to_string_lossy(), "rundll32");
        assert_eq!(args, vec!["url.dll,FileProtocolHandler", url]);
    }

    #[test]
    fn discovery_validation_rejects_stale_or_non_xai_endpoints() {
        validate_discovery(&DiscoveryDocument {
            issuer: ISSUER.to_string(),
            authorization_endpoint: "https://auth.x.ai/oauth/authorize".to_string(),
            token_endpoint: "https://auth.x.ai/oauth/token".to_string(),
            device_authorization_endpoint: None,
        })
        .unwrap();

        let stale = validate_discovery(&DiscoveryDocument {
            issuer: ISSUER.to_string(),
            authorization_endpoint: "https://example.com/oauth/authorize".to_string(),
            token_endpoint: "https://auth.x.ai/oauth/token".to_string(),
            device_authorization_endpoint: None,
        })
        .unwrap_err()
        .to_string();
        assert!(stale.contains("x.ai"));

        let insecure = validate_xai_https_endpoint("http://auth.x.ai/oauth/token")
            .unwrap_err()
            .to_string();
        assert!(insecure.contains("https"));
    }

    #[test]
    fn token_response_parses_and_extracts_email_claim() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"email":"pz@example.com"}"#);
        let id_token = format!("header.{payload}.sig");
        let tokens = tokens_from_response(TokenResponse {
            id_token: id_token.clone(),
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 60,
            token_type: "Bearer".to_string(),
        })
        .unwrap();

        assert_eq!(tokens.access, "access");
        assert_eq!(tokens.refresh, "refresh");
        assert_eq!(tokens.id_token, id_token);
        assert_eq!(tokens.email, "pz@example.com");
        assert!(tokens.expires > now_millis());
    }

    #[test]
    fn token_response_parse_error_redacts_secret_material() {
        let raw =
            r#"{"access_token":"secret-access","refresh_token":"secret-refresh"}{"extra":true}"#;
        let err = parse_token_response(raw).unwrap_err().to_string();

        assert!(err.contains("supergrok token response was not valid JSON"));
        assert!(err.contains("[redacted]"));
        assert!(!err.contains("secret-access"));
        assert!(!err.contains("secret-refresh"));
    }
}
