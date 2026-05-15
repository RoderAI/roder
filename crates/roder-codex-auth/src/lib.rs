use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::RngExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const ORIGINATOR: &str = "codex_cli_rs";
const REFRESH_EXPIRY_SKEW_MILLIS: i64 = 3 * 60 * 1000;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
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
    pub account_id: String,
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
    account_id: String,
    #[serde(default)]
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct Claims {
    #[serde(default)]
    chatgpt_account_id: String,
    #[serde(default)]
    organizations: Vec<OrgClaim>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    openai_auth: OpenAIAuthClaim,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAIAuthClaim {
    #[serde(default)]
    chatgpt_account_id: String,
}

#[derive(Debug, Deserialize)]
struct OrgClaim {
    id: String,
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
        load_tokens_from(&self.path()).or_else(|err| {
            if self.path().exists() {
                return Err(err);
            }
            load_tokens_from(&gode_auth_path())
        })
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
        self.data_dir.join("auth").join("codex.json")
    }
}

pub async fn access_token() -> anyhow::Result<Option<(String, Option<String>)>> {
    let store = Store::new();
    let tokens = store.load()?;
    if tokens.refresh.trim().is_empty() && tokens.access.trim().is_empty() {
        return Ok(None);
    }
    let now = now_millis();
    if !tokens.access.trim().is_empty() && tokens.expires > now + REFRESH_EXPIRY_SKEW_MILLIS {
        return Ok(Some((tokens.access, non_empty(tokens.account_id))));
    }
    if tokens.refresh.trim().is_empty() {
        return Ok(None);
    }
    let mut refreshed = refresh(&tokens.refresh).await?;
    if refreshed.account_id.is_empty() {
        refreshed.account_id = tokens.account_id;
    }
    if refreshed.refresh.is_empty() {
        refreshed.refresh = tokens.refresh;
    }
    let access = refreshed.access.clone();
    let account_id = non_empty(refreshed.account_id.clone());
    store.save(refreshed)?;
    Ok(Some((access, account_id)))
}

pub async fn login() -> anyhow::Result<Tokens> {
    let pkce_verifier = random_string(43);
    let pkce_challenge = code_challenge(&pkce_verifier);
    let state = random_string(43);
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))?;
    let redirect_uri = format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}");
    let auth_url = authorize_url(&redirect_uri, &pkce_challenge, &state);

    open_browser(&auth_url)?;
    eprintln!("Codex sign-in URL: {auth_url}");

    let (code, returned_state) = wait_for_callback(listener)?;
    if returned_state != state {
        anyhow::bail!("invalid oauth state");
    }
    let tokens = exchange_code(&code, &redirect_uri, &pkce_verifier).await?;
    Store::new().save(tokens.clone())?;
    Ok(tokens)
}

pub async fn status() -> anyhow::Result<Option<Tokens>> {
    let tokens = Store::new().load()?;
    Ok((!tokens.refresh.trim().is_empty()).then_some(tokens))
}

pub fn logout() -> anyhow::Result<()> {
    Store::new().delete()
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
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    token_request(&params).await
}

async fn exchange_code(code: &str, redirect_uri: &str, verifier: &str) -> anyhow::Result<Tokens> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    token_request(&params).await
}

async fn token_request(params: &[(&str, &str)]) -> anyhow::Result<Tokens> {
    let response = Client::new()
        .post(TOKEN_ENDPOINT)
        .form(params)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("codex token request failed: {status} {}", text.trim());
    }
    let token_response: TokenResponse = serde_json::from_str(&text)?;
    tokens_from_response(token_response)
}

fn tokens_from_response(response: TokenResponse) -> anyhow::Result<Tokens> {
    if response.access_token.trim().is_empty() {
        anyhow::bail!("codex token response missing access_token");
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
            response.token_type.clone()
        },
        refresh: response.refresh_token.clone(),
        access: response.access_token.clone(),
        expires: now_millis() + expires_in * 1000,
        account_id: extract_account_id(&response)
            .or_else(|| non_empty(response.account_id.clone()))
            .unwrap_or_default(),
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
        "Connected to Codex. You can close this tab."
    } else {
        "Codex sign-in failed."
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn authorize_url(redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&state={}&originator={}",
        urlencoding::encode(CLIENT_ID),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(
            "openid profile email offline_access api.connectors.read api.connectors.invoke"
        ),
        urlencoding::encode(challenge),
        urlencoding::encode(state),
        urlencoding::encode(ORIGINATOR),
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

fn extract_account_id(response: &TokenResponse) -> Option<String> {
    for token in [&response.id_token, &response.access_token] {
        let Some(claims) = parse_claims(token) else {
            continue;
        };
        if !claims.chatgpt_account_id.is_empty() {
            return Some(claims.chatgpt_account_id);
        }
        if !claims.openai_auth.chatgpt_account_id.is_empty() {
            return Some(claims.openai_auth.chatgpt_account_id);
        }
        if let Some(org) = claims.organizations.first() {
            return Some(org.id.clone());
        }
    }
    None
}

fn parse_claims(token: &str) -> Option<Claims> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
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
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.arg("/C").arg("start");
        command
    };
    command.arg(url);
    let status = command.status()?;
    if !status.success() {
        anyhow::bail!("failed to open browser");
    }
    Ok(())
}

fn roder_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".roder")
}

fn gode_auth_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gode")
        .join("auth")
        .join("codex.json")
}

fn normalize(tokens: &mut Tokens) {
    if tokens.token_type.is_empty() {
        tokens.token_type = default_token_type();
    }
    tokens.refresh = tokens.refresh.trim().to_string();
    tokens.access = tokens.access.trim().to_string();
    tokens.account_id = tokens.account_id.trim().to_string();
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

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_account_id_from_jwt_claims() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"chatgpt_account_id":"acct_123"}"#);
        let token = format!("header.{payload}.sig");
        let response = TokenResponse {
            access_token: token,
            ..TokenResponse::default()
        };
        assert_eq!(extract_account_id(&response).as_deref(), Some("acct_123"));
    }

    #[test]
    fn authorization_url_matches_gode_oauth_shape() {
        let url = authorize_url("http://localhost:1455/auth/callback", "challenge", "state");
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("originator=codex_cli_rs"));
        assert!(url.contains("api.connectors.read"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("code_challenge_method=S256"));
    }
}
