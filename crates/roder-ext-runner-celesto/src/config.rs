use std::fmt;

use roder_api::remote_runner::{RunnerDestination, RunnerSessionState};

pub const PROVIDER_ID: &str = "celesto";
pub const CELESTO_EXTENSION_ID: &str = "roder-ext-runner-celesto";
pub const TOKEN_ENV: &str = "CELESTO_API_KEY";
pub const RODER_TOKEN_ENV: &str = "RODER_CELESTO_API_KEY";
pub const BASE_URL_ENV: &str = "CELESTO_BASE_URL";
pub const RODER_BASE_URL_ENV: &str = "RODER_CELESTO_BASE_URL";
pub const LIVE_ENV: &str = "RODER_LIVE_CELESTO_RUNNER";
pub const DEFAULT_BASE_URL: &str = "https://api.celesto.ai";
pub const DEFAULT_WORKING_DIR: &str = "/root/roder";
pub const DEFAULT_TEMPLATE_ID: &str = "coding-agent";

/// What to do with the Celesto computer when the runner session closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupMode {
    /// Delete the computer (default for computers Roder created).
    DeleteOnClose,
    /// Stop the computer but keep its files for a later resume.
    StopOnClose,
    /// Leave the computer running (default when reusing an existing one).
    Keep,
}

impl CleanupMode {
    pub fn parse(raw: Option<&str>, reuse_existing: bool) -> anyhow::Result<Self> {
        match raw.map(str::trim) {
            None | Some("") => Ok(if reuse_existing {
                Self::Keep
            } else {
                Self::DeleteOnClose
            }),
            Some("delete-on-close") | Some("delete") => Ok(Self::DeleteOnClose),
            Some("stop-on-close") | Some("stop") => Ok(Self::StopOnClose),
            Some("keep") => Ok(Self::Keep),
            Some(other) => anyhow::bail!(
                "unknown celesto cleanup mode '{other}' (expected delete-on-close, stop-on-close, or keep)"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeleteOnClose => "delete-on-close",
            Self::StopOnClose => "stop-on-close",
            Self::Keep => "keep",
        }
    }
}

/// Resolved configuration for a Celesto runner session. The token is never
/// printed in `Debug` output so it cannot leak into logs or session state.
#[derive(Clone)]
pub struct CelestoConfig {
    pub token: String,
    pub base_url: String,
    pub computer_id: Option<String>,
    pub template_id: String,
    pub template_version: Option<String>,
    pub cpus: Option<u32>,
    pub memory: Option<u32>,
    pub disk_size_mb: Option<u32>,
    pub working_dir: String,
    pub cleanup: CleanupMode,
    pub metadata: serde_json::Value,
}

impl fmt::Debug for CelestoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CelestoConfig")
            .field("token", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("computer_id", &self.computer_id)
            .field("template_id", &self.template_id)
            .field("template_version", &self.template_version)
            .field("cpus", &self.cpus)
            .field("memory", &self.memory)
            .field("disk_size_mb", &self.disk_size_mb)
            .field("working_dir", &self.working_dir)
            .field("cleanup", &self.cleanup)
            .finish()
    }
}

impl CelestoConfig {
    pub fn from_destination(destination: &RunnerDestination) -> anyhow::Result<Self> {
        let config = &destination.config;
        let token = resolve_token(config)?;
        let base_url = resolve_base_url(string_field(config, "base_url"));
        let computer_id = string_field(config, "computer_id");
        let template_id =
            string_field(config, "template_id").unwrap_or_else(|| DEFAULT_TEMPLATE_ID.to_string());
        let working_dir =
            string_field(config, "working_dir").unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string());
        let cleanup = CleanupMode::parse(
            string_field(config, "cleanup").as_deref(),
            computer_id.is_some(),
        )?;
        Ok(Self {
            token,
            base_url,
            computer_id,
            template_id,
            template_version: string_field(config, "template_version"),
            cpus: u32_field(config, "cpus"),
            memory: u32_field(config, "memory"),
            disk_size_mb: u32_field(config, "disk_size_mb"),
            working_dir,
            cleanup,
            metadata: config
                .get("metadata")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        })
    }

    pub fn from_state(state: &RunnerSessionState) -> anyhow::Result<Self> {
        let token = env_nonempty(RODER_TOKEN_ENV)
            .or_else(|| env_nonempty(TOKEN_ENV))
            .ok_or_else(|| {
                anyhow::anyhow!("celesto runner resume requires {TOKEN_ENV} (or {RODER_TOKEN_ENV})")
            })?;
        let metadata = &state.metadata;
        let base_url = resolve_base_url(string_field(metadata, "base_url"));
        let computer_id = string_field(metadata, "computer_id");
        let working_dir =
            string_field(metadata, "working_dir").unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string());
        let cleanup = CleanupMode::parse(string_field(metadata, "cleanup").as_deref(), true)?;
        Ok(Self {
            token,
            base_url,
            computer_id,
            template_id: string_field(metadata, "template_id")
                .unwrap_or_else(|| DEFAULT_TEMPLATE_ID.to_string()),
            template_version: None,
            cpus: None,
            memory: None,
            disk_size_mb: None,
            working_dir,
            cleanup,
            metadata: serde_json::Value::Null,
        })
    }
}

fn resolve_token(config: &serde_json::Value) -> anyhow::Result<String> {
    env_nonempty(RODER_TOKEN_ENV)
        .or_else(|| env_nonempty(TOKEN_ENV))
        .or_else(|| string_field(config, "token"))
        .or_else(|| string_field(config, "api_key"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "celesto runner requires {TOKEN_ENV} (or {RODER_TOKEN_ENV}) or a destination config token"
            )
        })
}

fn resolve_base_url(config_value: Option<String>) -> String {
    env_nonempty(RODER_BASE_URL_ENV)
        .or_else(|| env_nonempty(BASE_URL_ENV))
        .or(config_value)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn string_field(config: &serde_json::Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn u32_field(config: &serde_json::Value, key: &str) -> Option<u32> {
    config
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}
