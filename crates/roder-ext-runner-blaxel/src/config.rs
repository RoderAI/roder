use std::fmt;

use anyhow::Context;
use roder_api::remote_runner::{RunnerDestination, RunnerSessionState};
use serde::Deserialize;
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
pub const MAX_STANDBY_AFTER_SECONDS: u64 = 24 * 60 * 60;

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

/// Supported Blaxel lifecycle expiration conditions. Every condition deletes
/// the sandbox when it becomes true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpirationPolicyType {
    /// Delete after the sandbox has not been used for the configured duration.
    TtlIdle,
    /// Delete after the configured duration from sandbox creation.
    TtlMaxAge,
}

impl ExpirationPolicyType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TtlIdle => "ttl-idle",
            Self::TtlMaxAge => "ttl-max-age",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ExpirationPolicyAction {
    #[default]
    Delete,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpirationPolicyInput {
    #[serde(rename = "type")]
    kind: ExpirationPolicyType,
    value: String,
    #[serde(default, rename = "action")]
    _action: ExpirationPolicyAction,
}

/// A validated Blaxel sandbox expiration policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpirationPolicy {
    pub kind: ExpirationPolicyType,
    pub value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxLifecycleInput {
    #[serde(default)]
    expiration_policies: Vec<ExpirationPolicyInput>,
}

/// Lifecycle policy managed by this runner. Absence means the runner leaves an
/// existing sandbox's lifecycle configuration unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxLifecycle {
    pub expiration_policies: Vec<ExpirationPolicy>,
}

impl SandboxLifecycle {
    pub(crate) fn api_value(&self) -> Value {
        let policies: Vec<_> = self
            .expiration_policies
            .iter()
            .map(|policy| {
                serde_json::json!({
                    "action": "delete",
                    "type": policy.kind.as_str(),
                    "value": policy.value,
                })
            })
            .collect();
        serde_json::json!({ "expirationPolicies": policies })
    }

    pub(crate) fn config_value(&self) -> Value {
        let policies: Vec<_> = self
            .expiration_policies
            .iter()
            .map(|policy| {
                serde_json::json!({
                    "type": policy.kind.as_str(),
                    "value": policy.value,
                })
            })
            .collect();
        serde_json::json!({ "expiration_policies": policies })
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
    /// Additional ports are immutable runtime configuration, declared at creation.
    pub ports: Vec<crate::SandboxPort>,
    pub region: Option<String>,
    /// Immutable dedicated outbound gateway; absence uses shared egress.
    pub egress_gateway: Option<String>,
    /// Workspace frozen with a dedicated assignment; credentials may rotate,
    /// but an identically named gateway in another workspace is not the same IP.
    pub egress_workspace: Option<String>,
    pub ttl: Option<String>,
    /// Seconds to keep a bounded process lease alive after each runner operation.
    pub standby_after_seconds: Option<u64>,
    /// Server-side deletion policies reconciled on both create and rejoin.
    pub lifecycle: Option<SandboxLifecycle>,
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

fn strict_optional_string(config: &Value, key: &str) -> anyhow::Result<Option<String>> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("blaxel runner `{key}` must be a duration string"))?
        .trim();
    anyhow::ensure!(!value.is_empty(), "blaxel runner `{key}` cannot be empty");
    Ok(Some(value.to_string()))
}

pub(crate) fn duration_seconds(value: &str, field: &str) -> anyhow::Result<u64> {
    let unit = value
        .chars()
        .last()
        .ok_or_else(|| anyhow::anyhow!("blaxel runner `{field}` cannot be empty"))?;
    let amount = &value[..value.len() - unit.len_utf8()];
    anyhow::ensure!(
        !amount.is_empty() && amount.bytes().all(|byte| byte.is_ascii_digit()),
        "blaxel runner `{field}` must be an integer duration using s, m, h, d, or w"
    );
    let amount = amount
        .parse::<u64>()
        .with_context(|| format!("parse blaxel runner `{field}` duration"))?;
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => anyhow::bail!(
            "blaxel runner `{field}` must be an integer duration using s, m, h, d, or w"
        ),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("blaxel runner `{field}` duration is too large"))?;
    Ok(seconds)
}

fn parse_standby_after(config: &Value) -> anyhow::Result<Option<u64>> {
    let Some(value) = strict_optional_string(config, "standby_after")? else {
        return Ok(None);
    };
    let seconds = duration_seconds(&value, "standby_after")?;
    if seconds == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        seconds <= MAX_STANDBY_AFTER_SECONDS,
        "blaxel runner `standby_after` cannot exceed 24h"
    );
    Ok(Some(seconds))
}

fn parse_lifecycle(config: &Value) -> anyhow::Result<Option<SandboxLifecycle>> {
    let Some(value) = config.get("lifecycle") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let input: SandboxLifecycleInput = serde_json::from_value(value.clone())
        .context("parse blaxel runner `lifecycle` configuration")?;
    let mut expiration_policies = Vec::with_capacity(input.expiration_policies.len());
    for policy in input.expiration_policies {
        anyhow::ensure!(
            !expiration_policies
                .iter()
                .any(|existing: &ExpirationPolicy| existing.kind == policy.kind),
            "blaxel runner `lifecycle` contains duplicate `{}` policies",
            policy.kind.as_str()
        );
        let value = policy.value.trim();
        let seconds = duration_seconds(value, policy.kind.as_str())?;
        anyhow::ensure!(
            seconds > 0,
            "blaxel runner lifecycle `{}` duration must be positive",
            policy.kind.as_str()
        );
        expiration_policies.push(ExpirationPolicy {
            kind: policy.kind,
            value: value.to_string(),
        });
    }
    Ok(Some(SandboxLifecycle {
        expiration_policies,
    }))
}

impl BlaxelConfig {
    fn resolve(
        config: &Value,
        default_cleanup: CleanupMode,
        profile: Option<&crate::BlaxelWorkspace>,
    ) -> anyhow::Result<Self> {
        let (token, workspace, base_url) = if let Some(profile) = profile {
            profile.assert_scope(config)?;
            (
                profile.token.clone(),
                Some(profile.workspace.clone()),
                profile.base_url.clone(),
            )
        } else {
            anyhow::ensure!(
                config.get("explicit_workspace").is_none(),
                "explicit workspace state requires an explicit workspace provider"
            );
            // Default runners continue to pick up operator credential rotation.
            let token = first_env(&[RODER_TOKEN_ENV, TOKEN_ENV, BL_TOKEN_ENV])
                .or_else(|| string_field(config, "token"))
                .or_else(|| string_field(config, "api_key"))
                .ok_or_else(|| anyhow::anyhow!("blaxel runner requires {TOKEN_ENV} (or {BL_TOKEN_ENV}) or a destination config token"))?;
            let workspace = first_env(&[RODER_WORKSPACE_ENV, WORKSPACE_ENV, BLAXEL_WORKSPACE_ENV])
                .or_else(|| string_field(config, "workspace"));
            let base_url = first_env(&[RODER_BASE_URL_ENV, BASE_URL_ENV])
                .or_else(|| string_field(config, "base_url"))
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string();
            (Redacted(token), workspace, base_url)
        };
        let egress_gateway = crate::egress::gateway(config)?;
        let egress_workspace = if egress_gateway.is_some() {
            anyhow::ensure!(
                workspace.is_some(),
                "dedicated egress requires an explicit provider workspace"
            );
            if let Some(expected) = config.get("egress_workspace") {
                anyhow::ensure!(
                    workspace.as_deref() == expected.as_str(),
                    "dedicated egress provider workspace changed"
                );
            }
            workspace.clone()
        } else {
            None
        };

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

        let standby_after_seconds = parse_standby_after(config)?;
        let lifecycle = parse_lifecycle(config)?;

        Ok(Self {
            token,
            workspace,
            base_url,
            sandbox_name: string_field(config, "sandbox_name"),
            external_id: string_field(config, "external_id"),
            sandbox_name_prefix: string_field(config, "sandbox_name_prefix")
                .unwrap_or_else(|| "roder".to_string()),
            image: string_field(config, "image").unwrap_or_else(|| DEFAULT_IMAGE.to_string()),
            memory_mb,
            ports: crate::ports::parse_ports(config)?,
            region: string_field(config, "region"),
            egress_gateway,
            egress_workspace,
            ttl: string_field(config, "ttl"),
            standby_after_seconds,
            lifecycle,
            working_dir: string_field(config, "working_dir")
                .unwrap_or_else(|| DEFAULT_WORKING_DIR.to_string()),
            cleanup,
        })
    }

    /// Resolve from a destination chosen at thread creation. Fresh sandboxes
    /// default to `delete-on-close`; reusing an existing `sandbox_name`
    /// defaults to `detach-on-close`.
    pub fn from_destination(destination: &RunnerDestination) -> anyhow::Result<Self> {
        Self::from_destination_with_workspace(destination, None)
    }

    /// Resolve runtime settings with a separately selected account. The account
    /// is checked against any persisted scope before using its credentials.
    pub fn from_destination_in_workspace(
        destination: &RunnerDestination,
        workspace: &crate::BlaxelWorkspace,
    ) -> anyhow::Result<Self> {
        Self::from_destination_with_workspace(destination, Some(workspace))
    }

    fn from_destination_with_workspace(
        destination: &RunnerDestination,
        profile: Option<&crate::BlaxelWorkspace>,
    ) -> anyhow::Result<Self> {
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
        Self::resolve(&destination.config, default_cleanup, profile)
    }

    pub(crate) fn from_state_in_workspace(
        state: &RunnerSessionState,
        profile: &crate::BlaxelWorkspace,
    ) -> anyhow::Result<Self> {
        // Explicit profiles require persisted scope; never interpret old or
        // foreign state using whichever account the caller selected today.
        anyhow::ensure!(
            state
                .metadata
                .get("explicit_workspace")
                .and_then(Value::as_bool)
                == Some(true)
                && state.metadata.get("workspace").and_then(Value::as_str)
                    == Some(profile.workspace.as_str())
                && state.metadata.get("base_url").and_then(Value::as_str)
                    == Some(profile.base_url.as_str()),
            "session state does not belong to explicit workspace"
        );
        Self::resolve(&state.metadata, CleanupMode::DetachOnClose, Some(profile))
    }

    /// Resolve from persisted session state for resume/rejoin. The token always
    /// comes from the environment, never from state.
    pub fn from_state(state: &RunnerSessionState) -> anyhow::Result<Self> {
        let mut config = Self::resolve(&state.metadata, CleanupMode::DetachOnClose, None)?;
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

#[cfg(test)]
mod tests;
