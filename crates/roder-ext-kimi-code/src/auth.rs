use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::browser::open_browser;

const DEFAULT_OAUTH_HOST: &str = "https://auth.kimi.com";
pub const DEFAULT_MANAGED_BASE_URL: &str = "https://api.kimi.com/coding/v1";
pub const DEFAULT_OPEN_PLATFORM_BASE_URL: &str = "https://api.moonshot.ai/v1";
const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const KIMI_CODE_PLATFORM: &str = "kimi_code_cli";
const KIMI_CODE_USER_AGENT_PRODUCT: &str = "kimi-code-cli";
const REFRESH_EXPIRY_SKEW_MILLIS: i64 = 3 * 60 * 1000;
const RODER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn auth_client() -> Client {
    Client::builder()
        .user_agent(format!("Roder/{RODER_VERSION} (+https://roder.sh)"))
        .build()
        .expect("failed to construct reqwest client for Kimi Code auth")
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
    pub scope: String,
}

#[derive(Debug, Default, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    scope: String,
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
        self.data_dir.join("auth").join("kimi-code.json")
    }

    fn device_id_path(&self) -> PathBuf {
        self.data_dir.join("auth").join("kimi-code-device-id")
    }
}

pub fn managed_base_url() -> String {
    std::env::var("KIMI_CODE_BASE_URL")
        .ok()
        .or_else(|| std::env::var("RODER_KIMI_CODE_BASE_URL").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MANAGED_BASE_URL.to_string())
}

pub fn inference_headers() -> anyhow::Result<Vec<(String, String)>> {
    let mut headers = device_headers()?
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect::<Vec<_>>();
    headers.push(("User-Agent".to_string(), kimi_code_user_agent()));
    Ok(headers)
}

fn kimi_code_user_agent() -> String {
    format!("{KIMI_CODE_USER_AGENT_PRODUCT}/{RODER_VERSION} (roder)")
}

pub fn oauth_host() -> String {
    std::env::var("KIMI_CODE_OAUTH_HOST")
        .ok()
        .or_else(|| std::env::var("KIMI_OAUTH_HOST").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_OAUTH_HOST.to_string())
}

pub fn has_stored_tokens() -> bool {
    Store::new()
        .load()
        .ok()
        .is_some_and(|tokens| !tokens.refresh.trim().is_empty() || !tokens.access.trim().is_empty())
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
    if refreshed.scope.is_empty() {
        refreshed.scope = tokens.scope;
    }
    let access = refreshed.access.clone();
    store.save(refreshed)?;
    Ok(Some(access))
}

pub async fn device_flow() -> anyhow::Result<Tokens> {
    let oauth_host = oauth_host();
    let device_endpoint = device_authorization_url(&oauth_host)?;
    let token_endpoint = token_url(&oauth_host)?;

    let client = auth_client();
    let response = client
        .post(device_endpoint)
        .headers(device_header_map()?)
        .form(&[("client_id", CLIENT_ID)])
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "kimi-code device authorization request failed: {status} {}",
            text.trim()
        );
    }
    let device_auth: DeviceAuthorizationResponse = serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!("kimi-code device authorization response was not valid JSON: {e}\n{text}")
    })?;

    let verification_uri = device_auth
        .verification_uri_complete
        .as_ref()
        .unwrap_or(&device_auth.verification_uri);
    eprintln!("Kimi Code device sign-in");
    eprintln!("User code: {}", device_auth.user_code);
    eprintln!("Open: {verification_uri}");
    open_browser(verification_uri).or_else(|err| {
        eprintln!("Could not open browser automatically: {err}");
        Ok::<(), anyhow::Error>(())
    })?;

    let token = poll_device_token(
        &token_endpoint,
        &device_auth.device_code,
        device_auth.interval,
        device_auth.expires_in,
    )
    .await?;
    Store::new().save(token.clone())?;
    Ok(token)
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
            anyhow::bail!("kimi-code device sign-in expired");
        }
        let response = client
            .post(token_endpoint)
            .headers(device_header_map()?)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            let token_response = parse_token_response(&text)?;
            return tokens_from_response(token_response);
        }
        let error: DeviceTokenErrorResponse = match serde_json::from_str(&text) {
            Ok(error) => error,
            Err(_) => {
                anyhow::bail!(
                    "kimi-code device token request failed: {status} {}",
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
                anyhow::bail!("kimi-code device sign-in expired (expired_token)");
            }
            "access_denied" => {
                let desc = error
                    .error_description
                    .as_deref()
                    .unwrap_or("access denied");
                anyhow::bail!("kimi-code device sign-in denied: {desc}");
            }
            other => {
                let desc = error.error_description.as_deref().unwrap_or(other);
                anyhow::bail!("kimi-code device sign-in error: {desc}");
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

async fn refresh(refresh_token: &str) -> anyhow::Result<Tokens> {
    let token_endpoint = token_url(&oauth_host())?;
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    token_request(&token_endpoint, &params).await
}

async fn token_request(token_endpoint: &str, params: &[(&str, &str)]) -> anyhow::Result<Tokens> {
    validate_kimi_https_endpoint(token_endpoint)?;
    let response = auth_client()
        .post(token_endpoint)
        .headers(device_header_map()?)
        .form(params)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "kimi-code token request failed: {status} {}",
            redacted_body_excerpt(&text)
        );
    }
    let token_response = parse_token_response(&text)?;
    tokens_from_response(token_response)
}

fn device_authorization_url(oauth_host: &str) -> anyhow::Result<String> {
    let base = normalize_oauth_host(oauth_host)?;
    validate_kimi_https_endpoint(&base)?;
    Ok(format!("{base}/api/oauth/device_authorization"))
}

fn token_url(oauth_host: &str) -> anyhow::Result<String> {
    let base = normalize_oauth_host(oauth_host)?;
    validate_kimi_https_endpoint(&base)?;
    Ok(format!("{base}/api/oauth/token"))
}

fn normalize_oauth_host(oauth_host: &str) -> anyhow::Result<String> {
    Ok(oauth_host.trim_end_matches('/').to_string())
}

fn device_header_map() -> anyhow::Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in device_headers()? {
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| anyhow::anyhow!("invalid kimi-code header name: {err}"))?,
            reqwest::header::HeaderValue::from_str(&value).map_err(|err| {
                anyhow::anyhow!("invalid kimi-code header value for {name}: {err}")
            })?,
        );
    }
    Ok(headers)
}

fn device_headers() -> anyhow::Result<Vec<(&'static str, String)>> {
    let store = Store::new();
    Ok(vec![
        ("X-Msh-Platform", KIMI_CODE_PLATFORM.to_string()),
        ("X-Msh-Version", RODER_VERSION.to_string()),
        ("X-Msh-Device-Name", ascii_header(hostname(), "unknown")),
        (
            "X-Msh-Device-Model",
            ascii_header(device_model(), "unknown"),
        ),
        ("X-Msh-Os-Version", ascii_header(os_version(), "unknown")),
        ("X-Msh-Device-Id", read_or_create_device_id(&store)?),
    ])
}

fn read_or_create_device_id(store: &Store) -> anyhow::Result<String> {
    let path = store.device_id_path();
    if let Ok(contents) = fs::read_to_string(&path) {
        let id = contents.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    let id = Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{id}\n"))?;
    Ok(id)
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(version) = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
        {
            let text = String::from_utf8_lossy(&version.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }
    std::env::consts::OS.to_string()
}

fn device_model() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if os == "macos" {
        format!("macOS {} {arch}", os_version())
    } else if os == "windows" {
        format!("Windows {arch}")
    } else {
        format!("{os} {arch}")
    }
}

fn ascii_header(value: String, fallback: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|ch| (' '..='~').contains(ch))
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

fn validate_kimi_https_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let url = Url::parse(endpoint)?;
    if url.scheme() != "https" {
        anyhow::bail!("kimi-code oauth endpoint must use https");
    }
    let host = url.host_str().unwrap_or_default();
    if host == "kimi.com" || host.ends_with(".kimi.com") {
        return Ok(());
    }
    anyhow::bail!("kimi-code oauth endpoint must be hosted on kimi.com");
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

fn parse_token_response(text: &str) -> anyhow::Result<TokenResponse> {
    serde_json::from_str(text).map_err(|err| {
        anyhow::anyhow!(
            "kimi-code token response was not valid JSON: {err}; body: {}",
            redacted_body_excerpt(text)
        )
    })
}

fn tokens_from_response(response: TokenResponse) -> anyhow::Result<Tokens> {
    if response.access_token.trim().is_empty() {
        anyhow::bail!("kimi-code token response missing access_token");
    }
    if response.refresh_token.trim().is_empty() {
        anyhow::bail!("kimi-code token response missing refresh_token");
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
        scope: response.scope,
    };
    normalize(&mut tokens);
    Ok(tokens)
}

fn redacted_body_excerpt(body: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 1_000;
    let mut excerpt = body.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    if body.chars().count() > MAX_ERROR_BODY_CHARS {
        excerpt.push_str(" ...");
    }
    for field in ["access_token", "refresh_token", "access", "refresh"] {
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
    tokens.scope = tokens.scope.trim().to_string();
}

fn default_token_type() -> String {
    "Bearer".to_string()
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
    fn oauth_host_defaults_to_auth_kimi_com() {
        assert_eq!(oauth_host(), DEFAULT_OAUTH_HOST);
    }

    #[test]
    fn device_authorization_and_token_urls_use_kimi_host() {
        let device = device_authorization_url(DEFAULT_OAUTH_HOST).unwrap();
        let token = token_url(DEFAULT_OAUTH_HOST).unwrap();
        assert_eq!(
            device,
            "https://auth.kimi.com/api/oauth/device_authorization"
        );
        assert_eq!(token, "https://auth.kimi.com/api/oauth/token");
    }

    #[test]
    fn endpoint_validation_rejects_non_kimi_hosts() {
        let err = validate_kimi_https_endpoint("https://example.com/oauth/token")
            .unwrap_err()
            .to_string();
        assert!(err.contains("kimi.com"));

        let insecure = validate_kimi_https_endpoint("http://auth.kimi.com/oauth/token")
            .unwrap_err()
            .to_string();
        assert!(insecure.contains("https"));
    }

    #[test]
    fn token_response_parses_required_fields() {
        let tokens = tokens_from_response(TokenResponse {
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            expires_in: 120,
            token_type: "Bearer".to_string(),
            scope: "kimi".to_string(),
        })
        .unwrap();

        assert_eq!(tokens.access, "access");
        assert_eq!(tokens.refresh, "refresh");
        assert_eq!(tokens.scope, "kimi");
        assert!(tokens.expires > now_millis());
    }

    #[test]
    fn token_response_parse_error_redacts_secret_material() {
        let raw =
            r#"{"access_token":"secret-access","refresh_token":"secret-refresh"}{"extra":true}"#;
        let err = parse_token_response(raw).unwrap_err().to_string();

        assert!(err.contains("kimi-code token response was not valid JSON"));
        assert!(err.contains("[redacted]"));
        assert!(!err.contains("secret-access"));
        assert!(!err.contains("secret-refresh"));
    }

    #[test]
    fn device_headers_include_platform_and_version() {
        let headers = device_headers().unwrap();
        let platform = headers
            .iter()
            .find(|(name, _)| *name == "X-Msh-Platform")
            .map(|(_, value)| value.as_str());
        let version = headers
            .iter()
            .find(|(name, _)| *name == "X-Msh-Version")
            .map(|(_, value)| value.as_str());
        assert_eq!(platform, Some(KIMI_CODE_PLATFORM));
        assert_eq!(version, Some(RODER_VERSION));
    }

    #[test]
    fn inference_headers_include_kimi_code_cli_user_agent() {
        let headers = inference_headers().unwrap();
        let user_agent = headers
            .iter()
            .find(|(name, _)| name == "User-Agent")
            .map(|(_, value)| value.as_str());
        assert_eq!(user_agent, Some(kimi_code_user_agent().as_str()));
        assert!(headers.iter().any(|(name, _)| name == "X-Msh-Device-Id"));
    }

    #[test]
    fn managed_base_url_defaults_to_kimi_coding_api() {
        assert_eq!(managed_base_url(), DEFAULT_MANAGED_BASE_URL);
    }
}
