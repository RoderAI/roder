use std::fmt;

use roder_api::remote_runner::{RunnerDestination, RunnerSessionState};
use serde_json::Value;

pub const PROVIDER_ID: &str = "blaxel";
pub const EXTENSION_ID: &str = "roder-ext-runner-blaxel";

/// Documented credential env var (also accepts the Blaxel SDK's `BL_API_KEY`).
pub const TOKEN_ENV: &str = "BLAXEL_API_KEY";
pub const RODER_TOKEN_ENV: &str = "RODER_BLAXEL_API_KEY";
pub const BL_TOKEN_ENV: &str = "BL_API_KEY";

pub const WORKSPACE_ENV: &str = "BL_WORKSPACE";
pub const RODER_WORKSPACE_ENV: &str = "RODER_BLAXEL_WORKSPACE";
pub const BLAXEL_WORKSPACE_ENV: &str = "BLAXEL_WORKSPACE";

pub const BASE_URL_ENV: &str = "BLAXEL_RUNNER_BASE_URL";
pub const RODER_BASE_URL_ENV: &str = "RODER_BLAXEL_BASE_URL";

pub const LIVE_ENV: &str = "RODER_LIVE_BLAXEL_RUNNER";

pub const DEFAULT_BASE_URL: &str = "https://api.blaxel.ai/v0";
pub const DEFAULT_IMAGE: &str = "blaxel/base-image:latest";
pub const DEFAULT_MEMORY_MB: u32 = 4096;
pub const DEFAULT_WORKING_DIR: &str = "/home/user/roder";

/// Wrapper that keeps the API key out of `Debug`/log output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Redacted(pub String);

impl Redacted {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Lifecycle behavior applied when a session is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupMode {
    /// Permanently delete the sandbox on close (default for fresh sandboxes).
    DeleteOnClose,
    /// Leave the sandbox alive (scaling to standby) so it can be rejoined
    /// (default when reusing an existing sandbox).
    DetachOnClose,
    /// Never delete; identical to `DetachOnClose` but explicit and sticky.
    Keep,
}

impl CleanupMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CleanupMode::DeleteOnClose => "delete-on-close",
            CleanupMode::DetachOnClose => "detach-on-close",
            CleanupMode::Keep => "keep",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "delete-on-close" => Some(CleanupMode::DeleteOnClose),
            "detach-on-close" => Some(CleanupMode::DetachOnClose),
            "keep" => Some(CleanupMode::Keep),
            _ => None,
        }
    }

    /// Whether `close` should delete the sandbox.
    pub fn deletes_on_close(self) -> bool {
        matches!(self, CleanupMode::DeleteOnClose)
    }
}

/// Resolved configuration for a Blaxel sandbox session. The API key is stored in
/// a [`Redacted`] wrapper and never appears in `Debug` output, logs, or
/// serialized session state.
#[derive(Debug, Clone)]
pub struct BlaxelConfig {
    pub token: Redacted,
    pub workspace: Option<String>,
    pub base_url: String,
    /// Existing sandbox name to reuse, if any.
    pub sandbox_name: Option<String>,
    /// Caller-owned external id used to recover the sandbox on rejoin.
    pub external_id: Option<String>,
    /// Prefix used when generating a fresh sandbox name.
    pub sandbox_name_prefix: String,
    pub image: String,
    pub memory_mb: u32,
    pub region: Option<String>,
    pub ttl: Option<String>,
    pub working_dir: String,
    pub cleanup: CleanupMode,
}

fn string_field(config: &Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn first_env(vars: &[&str]) -> Option<String> {
    vars.iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

impl BlaxelConfig {
    fn resolve(config: &Value, default_cleanup: CleanupMode) -> anyhow::Result<Self> {
        // Environment overrides win over persisted/destination config so a
        // rotated key or relocated endpoint takes effect without rewriting
        // thread state.
        let token = first_env(&[RODER_TOKEN_ENV, TOKEN_ENV, BL_TOKEN_ENV])
            .or_else(|| string_field(config, "token"))
            .or_else(|| string_field(config, "api_key"))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "blaxel runner requires {TOKEN_ENV} (or {BL_TOKEN_ENV}) or a destination config token"
                )
            })?;

        let workspace = first_env(&[RODER_WORKSPACE_ENV, WORKSPACE_ENV, BLAXEL_WORKSPACE_ENV])
            .or_else(|| string_field(config, "workspace"));

        let base_url = first_env(&[RODER_BASE_URL_ENV, BASE_URL_ENV])
            .or_else(|| string_field(config, "base_url"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        let cleanup = config
            .get("cleanup")
            .and_then(Value::as_str)
            .and_then(CleanupMode::parse)
            .unwrap_or(default_cleanup);

        let memory_mb = config
            .get("memory")
            .and_then(Value::as_u64)
            .map(|value| value as u32)
            .unwrap_or(DEFAULT_MEMORY_MB);

        Ok(Self {
            token: Redacted(token),
            workspace,
            base_url,
            sandbox_name: string_field(config, "sandbox_name"),
            external_id: string_field(config, "external_id"),
            sandbox_name_prefix: string_field(config, "sandbox_name_prefix")
                .unwrap_or_else(|| "roder".to_string()),
            image: string_field(config, "image").unwrap_or_else(|| DEFAULT_IMAGE.to_string()),
            memory_mb,
            region: string_field(config, "region"),
            ttl: string_field(config, "ttl"),
            working_dir: string_field(config, "working_dir")
                .unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string()),
            cleanup,
        })
    }

    /// Resolve from a destination chosen at thread creation. Fresh sandboxes
    /// default to `delete-on-close`; reusing an existing `sandbox_name`
    /// defaults to `detach-on-close`.
    pub fn from_destination(destination: &RunnerDestination) -> anyhow::Result<Self> {
        let reusing_existing = destination
            .config
            .get("sandbox_name")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let default_cleanup = if reusing_existing {
            CleanupMode::DetachOnClose
        } else {
            CleanupMode::DeleteOnClose
        };
        Self::resolve(&destination.config, default_cleanup)
    }

    /// Resolve from persisted session state for resume/rejoin. The token always
    /// comes from the environment, never from state.
    pub fn from_state(state: &RunnerSessionState) -> anyhow::Result<Self> {
        let mut config = Self::resolve(&state.metadata, CleanupMode::DetachOnClose)?;
        if let Some(name) = state
            .metadata
            .get("sandbox_name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            config.sandbox_name = Some(name.to_string());
        }
        Ok(config)
    }
}
