//! Google Application Default Credentials (ADC) token acquisition
//! (roadmap phase 69).
//!
//! Dependency-light ADC chain, resolved in order:
//!
//! 1. **Authorized-user ADC JSON** (`GOOGLE_APPLICATION_CREDENTIALS` or the
//!    well-known `~/.config/gcloud/application_default_credentials.json`):
//!    the stored refresh token is exchanged at the OAuth token endpoint —
//!    plain HTTPS form posts, no crypto dependencies, fully fake-testable.
//! 2. **gcloud CLI** (`gcloud auth application-default print-access-token`,
//!    binary overridable via `RODER_GCLOUD_BIN`).
//!
//! Service-account key JSON requires RS256 request signing, which would
//! pull a crypto dependency; it is rejected with an actionable message
//! pointing at authorized-user ADC or gcloud. Tokens are cached and
//! refreshed 60 seconds before expiry. Secrets never appear in errors.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Default OAuth token endpoint; overridable for offline tests.
const DEFAULT_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const EXPIRY_SLACK: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct AdcFile {
    #[serde(rename = "type")]
    kind: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Cached ADC token source.
pub struct AdcTokenSource {
    credentials_path: Option<PathBuf>,
    token_endpoint: String,
    gcloud_bin: String,
    cache: Mutex<Option<CachedToken>>,
}

impl Default for AdcTokenSource {
    fn default() -> Self {
        Self::from_env()
    }
}

impl AdcTokenSource {
    pub fn from_env() -> Self {
        Self {
            credentials_path: std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
                .ok()
                .map(PathBuf::from)
                .or_else(well_known_adc_path),
            token_endpoint: std::env::var("RODER_GOOGLE_TOKEN_ENDPOINT")
                .ok()
                .unwrap_or_else(|| DEFAULT_TOKEN_ENDPOINT.to_string()),
            gcloud_bin: std::env::var("RODER_GCLOUD_BIN")
                .ok()
                .unwrap_or_else(|| "gcloud".to_string()),
            cache: Mutex::new(None),
        }
    }

    /// Test/configuration constructor with explicit sources.
    pub fn new(
        credentials_path: Option<PathBuf>,
        token_endpoint: impl Into<String>,
        gcloud_bin: impl Into<String>,
    ) -> Self {
        Self {
            credentials_path,
            token_endpoint: token_endpoint.into(),
            gcloud_bin: gcloud_bin.into(),
            cache: Mutex::new(None),
        }
    }

    /// Whether any ADC source looks available without performing I/O that
    /// could block (used for auth-configured hints).
    pub fn looks_available(&self) -> bool {
        self.credentials_path
            .as_ref()
            .is_some_and(|path| path.is_file())
    }

    /// Returns a valid access token, refreshing when the cache is empty or
    /// near expiry.
    pub async fn access_token(&self) -> anyhow::Result<String> {
        if let Some(cached) = self.cache.lock().unwrap().as_ref()
            && Instant::now() + EXPIRY_SLACK < cached.expires_at
        {
            return Ok(cached.token.clone());
        }
        let (token, expires_in) = self.acquire().await?;
        let expires_at = Instant::now() + Duration::from_secs(expires_in.unwrap_or(3600));
        *self.cache.lock().unwrap() = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });
        Ok(token)
    }

    async fn acquire(&self) -> anyhow::Result<(String, Option<u64>)> {
        if let Some(path) = self.credentials_path.as_ref().filter(|path| path.is_file()) {
            let text = std::fs::read_to_string(path)?;
            let file: AdcFile = serde_json::from_str(&text)
                .map_err(|_| anyhow::anyhow!("{} is not valid ADC JSON", path.display()))?;
            match file.kind.as_deref() {
                Some("authorized_user") => return self.refresh_authorized_user(&file).await,
                Some("service_account") => anyhow::bail!(
                    "service-account ADC keys need RS256 signing, which Roder does not bundle; \
                     run `gcloud auth application-default login` for authorized-user ADC or \
                     provide RODER_GOOGLE_SPEECH_ACCESS_TOKEN / a gcloud CLI on PATH"
                ),
                other => anyhow::bail!(
                    "unsupported ADC credential type {:?} in {}",
                    other.unwrap_or("missing"),
                    path.display()
                ),
            }
        }
        self.gcloud_print_access_token().await
    }

    async fn refresh_authorized_user(
        &self,
        file: &AdcFile,
    ) -> anyhow::Result<(String, Option<u64>)> {
        let (Some(client_id), Some(client_secret), Some(refresh_token)) = (
            file.client_id.as_deref(),
            file.client_secret.as_deref(),
            file.refresh_token.as_deref(),
        ) else {
            anyhow::bail!("authorized-user ADC JSON is missing client/refresh fields");
        };
        let response = reqwest::Client::new()
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            // Never echo response bodies: they can carry credential hints.
            anyhow::bail!("Google ADC token refresh failed with {status}");
        }
        let token: TokenResponse = response.json().await?;
        Ok((token.access_token, token.expires_in))
    }

    async fn gcloud_print_access_token(&self) -> anyhow::Result<(String, Option<u64>)> {
        let output = tokio::process::Command::new(&self.gcloud_bin)
            .args(["auth", "application-default", "print-access-token"])
            .output()
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "no ADC credentials file and the gcloud CLI ({}) is unavailable: {error}. \
                     Set RODER_GOOGLE_SPEECH_ACCESS_TOKEN, an API key, or run \
                     `gcloud auth application-default login`",
                    self.gcloud_bin
                )
            })?;
        if !output.status.success() {
            anyhow::bail!(
                "`{} auth application-default print-access-token` failed with {}",
                self.gcloud_bin,
                output.status
            );
        }
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        anyhow::ensure!(!token.is_empty(), "gcloud printed an empty access token");
        // gcloud tokens are typically valid for an hour; refreshing per the
        // default cache window is safe either way.
        Ok((token, Some(3600)))
    }
}

fn well_known_adc_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let path = home.join(".config/gcloud/application_default_credentials.json");
    path.is_file().then_some(path)
}
