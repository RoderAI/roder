use std::time::{Duration, Instant};

use serde::Deserialize;

/// roder.cloud team API keys are long-lived secrets with this prefix. They
/// are never sent to the inference edge directly; they are exchanged for
/// short-lived JWTs at the Rails web host.
pub const API_KEY_PREFIX: &str = "roder_";

/// Default dashboard / token-exchange host.
pub const DEFAULT_WEB_URL: &str = "https://roder.cloud";

const TOKEN_EXCHANGE_PATH: &str = "/api/v1/inference_tokens";

/// Refresh this long before the advertised expiry so in-flight requests never
/// race the 10-minute JWT TTL.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at: Instant,
}

impl CachedToken {
    fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

/**
 * Exchanges a long-lived `roder_` team API key for short-lived inference
 * JWTs at `{web_url}/api/v1/inference_tokens`, caching the JWT until close
 * to expiry. Shared across turns; cheap to clone behind an `Arc`.
 */
pub struct RoderCloudTokenSource {
    client: reqwest::Client,
    web_url: String,
    api_key: String,
    cached: tokio::sync::Mutex<Option<CachedToken>>,
}

impl RoderCloudTokenSource {
    pub fn new(web_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            web_url: web_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// Return a JWT for the inference edge, exchanging the API key only when
    /// no fresh cached token exists.
    pub async fn token(&self) -> anyhow::Result<String> {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref().filter(|token| token.is_fresh()) {
            return Ok(token.token.clone());
        }
        let fresh = self.exchange().await?;
        let token = fresh.token.clone();
        *cached = Some(fresh);
        Ok(token)
    }

    /// Drop any cached JWT and exchange again. Used after the inference edge
    /// rejects a token with `invalid_token`.
    pub async fn refresh(&self) -> anyhow::Result<String> {
        let mut cached = self.cached.lock().await;
        let fresh = self.exchange().await?;
        let token = fresh.token.clone();
        *cached = Some(fresh);
        Ok(token)
    }

    async fn exchange(&self) -> anyhow::Result<CachedToken> {
        let url = format!("{}{TOKEN_EXCHANGE_PATH}", self.web_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .send()
            .await
            .map_err(|err| anyhow::anyhow!("roder.cloud token exchange failed at {url}: {err}"))?;
        let status = response.status();
        let body = response.bytes().await.unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let detail = serde_json::from_slice::<TokenError>(&body)
                .map(|err| err.error)
                .unwrap_or_default();
            anyhow::bail!(
                "roder.cloud rejected the API key ({detail}); create a key at \
                 {}/teams and set RODER_CLOUD_API_KEY or [providers.roder-cloud].api_key",
                self.web_url
            );
        }
        if !status.is_success() {
            anyhow::bail!(
                "roder.cloud token exchange failed at {url}: HTTP {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let parsed: TokenResponse = serde_json::from_slice(&body).map_err(|err| {
            anyhow::anyhow!("roder.cloud token exchange returned malformed JSON: {err}")
        })?;
        if parsed.token.trim().is_empty() {
            anyhow::bail!("roder.cloud token exchange returned an empty token");
        }
        let ttl = Duration::from_secs(parsed.expires_in.max(1));
        let expires_at = Instant::now() + ttl.saturating_sub(EXPIRY_MARGIN).max(Duration::from_secs(1));
        Ok(CachedToken {
            token: parsed.token,
            expires_at,
        })
    }
}
