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
    pub region: Option<String>,
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

fn duration_seconds(value: &str, field: &str) -> anyhow::Result<u64> {
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

        let standby_after_seconds = parse_standby_after(config)?;
        let lifecycle = parse_lifecycle(config)?;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ExpirationPolicyType, parse_lifecycle, parse_standby_after};

    #[test]
    fn parses_standby_after_and_explicit_disable() {
        assert_eq!(
            parse_standby_after(&json!({ "standby_after": "5m" })).unwrap(),
            Some(300)
        );
        assert_eq!(
            parse_standby_after(&json!({ "standby_after": "300s" })).unwrap(),
            Some(300)
        );
        assert_eq!(
            parse_standby_after(&json!({ "standby_after": "0s" })).unwrap(),
            None
        );
    }

    #[test]
    fn standby_after_is_strict_and_bounded() {
        for invalid in [json!(300), json!("5 minutes"), json!("1.5h"), json!("25h")] {
            let error = parse_standby_after(&json!({ "standby_after": invalid })).unwrap_err();
            assert!(error.to_string().contains("standby_after"));
        }
    }

    #[test]
    fn parses_lifecycle_policy_and_emits_blaxel_shape() {
        let lifecycle = parse_lifecycle(&json!({
            "lifecycle": {
                "expiration_policies": [{ "type": "ttl-idle", "value": "7d" }]
            }
        }))
        .unwrap()
        .unwrap();

        assert_eq!(lifecycle.expiration_policies.len(), 1);
        assert_eq!(
            lifecycle.expiration_policies[0].kind,
            ExpirationPolicyType::TtlIdle
        );
        assert_eq!(
            lifecycle.api_value(),
            json!({
                "expirationPolicies": [{
                    "action": "delete",
                    "type": "ttl-idle",
                    "value": "7d"
                }]
            })
        );
        assert_eq!(
            lifecycle.config_value(),
            json!({
                "expiration_policies": [{ "type": "ttl-idle", "value": "7d" }]
            })
        );
    }

    #[test]
    fn lifecycle_rejects_unknown_keys_actions_and_duplicate_types() {
        let invalid = [
            json!({ "lifecycle": { "unknown": true } }),
            json!({
                "lifecycle": {
                    "expiration_policies": [{
                        "type": "ttl-idle",
                        "value": "7d",
                        "unknown": true
                    }]
                }
            }),
            json!({
                "lifecycle": {
                    "expiration_policies": [{
                        "type": "ttl-idle",
                        "value": "7d",
                        "action": "pause"
                    }]
                }
            }),
            json!({
                "lifecycle": {
                    "expiration_policies": [
                        { "type": "ttl-idle", "value": "7d" },
                        { "type": "ttl-idle", "value": "14d" }
                    ]
                }
            }),
        ];

        for config in invalid {
            assert!(parse_lifecycle(&config).is_err(), "accepted {config}");
        }
    }

    #[test]
    fn lifecycle_empty_policy_list_is_an_explicit_clear() {
        let lifecycle = parse_lifecycle(&json!({
            "lifecycle": { "expiration_policies": [] }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(lifecycle.api_value(), json!({ "expirationPolicies": [] }));
        assert!(parse_lifecycle(&json!({})).unwrap().is_none());
    }
}
