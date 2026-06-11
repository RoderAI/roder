use std::time::Duration;

use serde::Deserialize;

use crate::errors::redact_cursor_secrets;

const DEFAULT_BACKEND_BASE_URL: &str = "https://api2.cursor.sh";

#[derive(Debug, Clone, Default)]
pub struct CursorAuthConfig {
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub backend_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorAuthSource {
    EnvAccessToken,
    ApiKeyExchange,
}

impl CursorAuthSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EnvAccessToken => "env-access-token",
            Self::ApiKeyExchange => "api-key-exchange",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CursorAccessToken {
    pub token: String,
    pub source: CursorAuthSource,
}

impl CursorAuthConfig {
    pub fn has_auth(&self) -> bool {
        self.access_token().or_else(|| self.api_key()).is_some()
    }

    pub async fn resolve_access_token(&self) -> anyhow::Result<CursorAccessToken> {
        if let Some(token) = self.access_token() {
            return Ok(CursorAccessToken {
                token,
                source: CursorAuthSource::EnvAccessToken,
            });
        }
        let Some(api_key) = self.api_key() else {
            anyhow::bail!("Cursor API key is missing; set CURSOR_API_KEY or RODER_CURSOR_API_KEY");
        };
        let token = exchange_cursor_api_key(&self.backend_base_url(), &api_key).await?;
        Ok(CursorAccessToken {
            token,
            source: CursorAuthSource::ApiKeyExchange,
        })
    }

    fn api_key(&self) -> Option<String> {
        if let Some(key) = self.api_key.clone().and_then(nonempty) {
            return Some(key);
        }
        if let Some(key) = env_nonempty("CURSOR_API_KEY") {
            return Some(key);
        }
        if let Some(key) = env_nonempty("RODER_CURSOR_API_KEY") {
            return Some(key);
        }
        None
    }

    fn access_token(&self) -> Option<String> {
        self.access_token
            .clone()
            .and_then(nonempty)
            .or_else(|| env_nonempty("CURSOR_ACCESS_TOKEN"))
            .or_else(|| env_nonempty("CURSOR_AUTH_TOKEN"))
    }

    pub(crate) fn backend_base_url(&self) -> String {
        self.backend_base_url
            .clone()
            .and_then(nonempty)
            .or_else(|| env_nonempty("RODER_CURSOR_BACKEND_BASE_URL"))
            .or_else(|| env_nonempty("CURSOR_BACKEND_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_BACKEND_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct ExchangeResponse {
    #[serde(default, rename = "accessToken")]
    access_token_camel: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

async fn exchange_cursor_api_key(base_url: &str, api_key: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client
        .post(format!("{base_url}/auth/exchange_user_api_key"))
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Cursor API key exchange failed with HTTP {status}: {}",
            redact_cursor_secrets(&body)
        );
    }
    let payload: ExchangeResponse = response.json().await?;
    let token = payload
        .access_token_camel
        .or(payload.access_token)
        .or(payload.token)
        .and_then(nonempty);
    token.ok_or_else(|| anyhow::anyhow!("Cursor API key exchange did not return an access token"))
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(nonempty)
}

fn nonempty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_detects_configured_api_key() {
        let config = CursorAuthConfig {
            api_key: Some("crsr_test".to_string()),
            access_token: None,
            backend_base_url: None,
        };
        assert!(config.has_auth());
    }
}
