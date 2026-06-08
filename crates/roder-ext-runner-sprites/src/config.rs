use std::{collections::BTreeMap, fmt};

use anyhow::Context;
use roder_api::remote_runner::{RunnerDestination, RunnerSessionState};
use serde::{Deserialize, Serialize};

pub const PROVIDER_ID: &str = "sprites";
pub const SPRITES_EXTENSION_ID: &str = "roder-ext-runner-sprites";
pub const TOKEN_ENV: &str = "SPRITES_TOKEN";
pub const RODER_TOKEN_ENV: &str = "RODER_SPRITES_TOKEN";
pub const BASE_URL_ENV: &str = "SPRITES_BASE_URL";
pub const RODER_BASE_URL_ENV: &str = "RODER_SPRITES_BASE_URL";
pub const LIVE_ENV: &str = "RODER_LIVE_SPRITES_RUNNER";
pub const DEFAULT_BASE_URL: &str = "https://api.sprites.dev";
pub const DEFAULT_WORKING_DIR: &str = "/home/sprite/roder";
pub const DEFAULT_REMOTE_RODER_BASE_URL: &str = "https://dl.roder.sh/latest";
pub const DEFAULT_REMOTE_RODER_BINARY: &str = "remote-roder";
pub const DEFAULT_APP_SERVER_SERVICE_NAME: &str = "roder-app-server";
pub const DEFAULT_APP_SERVER_PORT: u16 = 17373;
pub const DEFAULT_APP_SERVER_TOKEN_ENV: &str = "RODER_REMOTE_APP_SERVER_TOKEN";

#[derive(Clone)]
pub struct SpritesConfig {
    pub token: String,
    pub base_url: String,
    pub sprite_name: Option<String>,
    pub sprite_name_prefix: String,
    pub url_auth: UrlAuth,
    pub cleanup: CleanupMode,
    pub working_dir: String,
    pub network_policy: Option<serde_json::Value>,
    pub privileges_policy: Option<serde_json::Value>,
    pub resources_policy: Option<serde_json::Value>,
    pub connectors: Vec<serde_json::Value>,
    pub labels: Vec<String>,
    pub metadata: serde_json::Value,
    pub app_server: Option<SpritesAppServerConfig>,
}

impl fmt::Debug for SpritesConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpritesConfig")
            .field("token", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("sprite_name", &self.sprite_name)
            .field("sprite_name_prefix", &self.sprite_name_prefix)
            .field("url_auth", &self.url_auth)
            .field("cleanup", &self.cleanup)
            .field("working_dir", &self.working_dir)
            .field(
                "network_policy",
                &redact_value(self.network_policy.as_ref()),
            )
            .field(
                "privileges_policy",
                &redact_value(self.privileges_policy.as_ref()),
            )
            .field(
                "resources_policy",
                &redact_value(self.resources_policy.as_ref()),
            )
            .field("connectors", &"<redacted>")
            .field("labels", &self.labels)
            .field("metadata", &redact_value(Some(&self.metadata)))
            .field("app_server", &self.app_server)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpritesAppServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_app_server_service_name")]
    pub service_name: String,
    #[serde(default = "default_app_server_port")]
    pub port: u16,
    #[serde(default = "default_remote_roder_base_url")]
    pub download_base_url: String,
    #[serde(default = "default_remote_roder_binary")]
    pub binary_name: String,
    #[serde(default = "default_remote_binary_path")]
    pub remote_binary_path: String,
    #[serde(default = "default_app_server_config_dir")]
    pub config_dir: String,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub local_binary_path: Option<String>,
    #[serde(default = "default_app_server_token_env")]
    pub auth_token_env: String,
    #[serde(default)]
    pub env_passthrough: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_true")]
    pub restart: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupMode {
    Keep,
    DeleteOnClose,
}

impl CleanupMode {
    fn parse(value: Option<&str>, has_user_sprite_name: bool) -> anyhow::Result<Self> {
        match value.unwrap_or(if has_user_sprite_name {
            "keep"
        } else {
            "delete-on-close"
        }) {
            "keep" => Ok(Self::Keep),
            "delete-on-close" => Ok(Self::DeleteOnClose),
            other => anyhow::bail!("unsupported sprites cleanup mode `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UrlAuth {
    Sprite,
    Public,
}

impl UrlAuth {
    fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.unwrap_or("sprite") {
            "sprite" => Ok(Self::Sprite),
            "public" => Ok(Self::Public),
            other => anyhow::bail!("unsupported sprites url_auth `{other}`"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sprite => "sprite",
            Self::Public => "public",
        }
    }
}

impl SpritesConfig {
    pub fn from_destination(destination: &RunnerDestination) -> anyhow::Result<Self> {
        let config = &destination.config;
        let token = resolve_token(config)?;
        let base_url = env_nonempty(RODER_BASE_URL_ENV)
            .or_else(|| env_nonempty(BASE_URL_ENV))
            .or_else(|| string_field(config, "base_url"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let sprite_name = string_field(config, "sprite_name");
        let sprite_name_prefix =
            string_field(config, "sprite_name_prefix").unwrap_or_else(|| "roder".to_string());
        let cleanup = CleanupMode::parse(
            string_field(config, "cleanup").as_deref(),
            sprite_name.is_some(),
        )?;
        let working_dir =
            string_field(config, "working_dir").unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string());
        let labels = config
            .get("labels")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
        let connectors = config
            .get("connectors")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(Self {
            token,
            base_url,
            sprite_name,
            sprite_name_prefix,
            url_auth: UrlAuth::parse(string_field(config, "url_auth").as_deref())?,
            cleanup,
            working_dir,
            network_policy: config.get("network_policy").cloned(),
            privileges_policy: config.get("privileges_policy").cloned(),
            resources_policy: config.get("resources_policy").cloned(),
            connectors,
            labels,
            metadata: config
                .get("metadata")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            app_server: parse_app_server_config(config)?,
        })
    }

    pub fn from_state(state: &RunnerSessionState) -> anyhow::Result<Self> {
        let token = env_nonempty(RODER_TOKEN_ENV)
            .or_else(|| env_nonempty(TOKEN_ENV))
            .ok_or_else(|| {
                anyhow::anyhow!("sprites runner resume requires {RODER_TOKEN_ENV} or {TOKEN_ENV}")
            })?;
        let base_url = env_nonempty(RODER_BASE_URL_ENV)
            .or_else(|| env_nonempty(BASE_URL_ENV))
            .or_else(|| string_field(&state.metadata, "base_url"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            token,
            base_url,
            sprite_name: string_field(&state.metadata, "sprite_name"),
            sprite_name_prefix: "roder".to_string(),
            url_auth: UrlAuth::Sprite,
            cleanup: CleanupMode::parse(string_field(&state.metadata, "cleanup").as_deref(), true)?,
            working_dir: string_field(&state.metadata, "working_dir")
                .unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string()),
            network_policy: None,
            privileges_policy: None,
            resources_policy: None,
            connectors: Vec::new(),
            labels: Vec::new(),
            metadata: serde_json::Value::Null,
            app_server: None,
        })
    }

    pub fn generated_sprite_name(&self) -> String {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        format!("{}-{}", self.sprite_name_prefix, &suffix[..12])
    }
}

fn parse_app_server_config(
    config: &serde_json::Value,
) -> anyhow::Result<Option<SpritesAppServerConfig>> {
    let Some(value) = config.get("app_server") else {
        return Ok(None);
    };
    if let Some(enabled) = value.as_bool() {
        return Ok(enabled.then(SpritesAppServerConfig::default));
    }
    let parsed = serde_json::from_value::<SpritesAppServerConfig>(value.clone())
        .context("parse sprites app_server config")?;
    Ok(parsed.enabled.then_some(parsed))
}

fn resolve_token(config: &serde_json::Value) -> anyhow::Result<String> {
    env_nonempty(RODER_TOKEN_ENV)
        .or_else(|| env_nonempty(TOKEN_ENV))
        .or_else(|| {
            string_field(config, "token_env")
                .or_else(|| string_field(config, "secret_env"))
                .and_then(|name| env_nonempty(&name))
        })
        .or_else(|| string_field(config, "token"))
        .with_context(|| {
            format!(
                "sprites runner requires {RODER_TOKEN_ENV}, {TOKEN_ENV}, or a destination token_env/secret_env reference"
            )
        })
}

fn string_field(config: &serde_json::Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn default_app_server_service_name() -> String {
    DEFAULT_APP_SERVER_SERVICE_NAME.to_string()
}

fn default_app_server_port() -> u16 {
    DEFAULT_APP_SERVER_PORT
}

fn default_remote_roder_base_url() -> String {
    DEFAULT_REMOTE_RODER_BASE_URL.to_string()
}

fn default_remote_roder_binary() -> String {
    DEFAULT_REMOTE_RODER_BINARY.to_string()
}

fn default_remote_binary_path() -> String {
    ".roder/bin/roder".to_string()
}

fn default_app_server_config_dir() -> String {
    ".roder/app-server".to_string()
}

fn default_app_server_token_env() -> String {
    DEFAULT_APP_SERVER_TOKEN_ENV.to_string()
}

fn default_true() -> bool {
    true
}

impl Default for SpritesAppServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            service_name: default_app_server_service_name(),
            port: default_app_server_port(),
            download_base_url: default_remote_roder_base_url(),
            binary_name: default_remote_roder_binary(),
            remote_binary_path: default_remote_binary_path(),
            config_dir: default_app_server_config_dir(),
            workspace_path: None,
            local_binary_path: None,
            auth_token_env: default_app_server_token_env(),
            env_passthrough: vec![
                "OPENAI_API_KEY".to_string(),
                "OPENAI_BASE_URL".to_string(),
                RODER_TOKEN_ENV.to_string(),
                TOKEN_ENV.to_string(),
            ],
            env: BTreeMap::new(),
            allowed_origins: Vec::new(),
            restart: true,
        }
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn redact_value(value: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    match value {
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    if secret_like_key(key) {
                        (
                            key.clone(),
                            serde_json::Value::String("<redacted>".to_string()),
                        )
                    } else {
                        (key.clone(), redact_value(Some(value)))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| redact_value(Some(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn redact_text(text: &str, token: &str) -> String {
    let mut out = text.replace(token, "<redacted>");
    for marker in ["authorization", "api_key", "secret", "token", "bearer"] {
        out = out.replace(marker, "<redacted>");
        out = out.replace(&marker.to_ascii_uppercase(), "<redacted>");
    }
    out
}

fn secret_like_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("secret")
        || key.contains("token")
        || key.contains("api_key")
        || key.contains("credential")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_nested_secret_values() {
        let redacted = redact_value(Some(&serde_json::json!({
            "connector": {"api_key": "plain"},
            "policy": {"allow": ["github.com"]}
        })));
        assert_eq!(redacted["connector"]["api_key"], "<redacted>");
        assert_eq!(redacted["policy"]["allow"][0], "github.com");
    }
}
