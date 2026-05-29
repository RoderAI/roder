use std::path::{Path, PathBuf};

use roder_api::dynamic_workflows::WorkflowRunLimits;
use serde::{Deserialize, Serialize};

use crate::Config;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DynamicWorkflowsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub trigger_word_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_with_ultracode: bool,
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: u32,
    #[serde(default = "default_max_agents_per_run")]
    pub max_agents_per_run: u32,
    #[serde(default = "default_agent_timeout_seconds")]
    pub default_agent_timeout_seconds: u64,
    #[serde(default = "default_run_timeout_seconds")]
    pub default_run_timeout_seconds: u64,
    #[serde(default = "default_checkpoint_bytes")]
    pub default_checkpoint_bytes: u64,
    #[serde(default = "default_max_report_bytes")]
    pub max_report_bytes: u64,
    #[serde(default = "default_workspace_workflows_dir")]
    pub workspace_workflows_dir: PathBuf,
    #[serde(default = "default_user_workflows_dir")]
    pub user_workflows_dir: PathBuf,
    #[serde(default)]
    pub approval: DynamicWorkflowApprovalConfig,
    #[serde(default)]
    pub live_checks: bool,
    #[serde(default)]
    pub deep_research_live: bool,
}

impl Default for DynamicWorkflowsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_word_enabled: true,
            auto_with_ultracode: true,
            max_concurrent_agents: default_max_concurrent_agents(),
            max_agents_per_run: default_max_agents_per_run(),
            default_agent_timeout_seconds: default_agent_timeout_seconds(),
            default_run_timeout_seconds: default_run_timeout_seconds(),
            default_checkpoint_bytes: default_checkpoint_bytes(),
            max_report_bytes: default_max_report_bytes(),
            workspace_workflows_dir: default_workspace_workflows_dir(),
            user_workflows_dir: default_user_workflows_dir(),
            approval: DynamicWorkflowApprovalConfig::default(),
            live_checks: false,
            deep_research_live: false,
        }
    }
}

impl DynamicWorkflowsConfig {
    pub fn limits(&self) -> WorkflowRunLimits {
        WorkflowRunLimits {
            max_concurrent_agents: self.max_concurrent_agents,
            max_agents_per_run: self.max_agents_per_run,
            default_agent_timeout_seconds: self.default_agent_timeout_seconds,
            default_run_timeout_seconds: self.default_run_timeout_seconds,
            default_checkpoint_bytes: self.default_checkpoint_bytes,
            max_report_bytes: self.max_report_bytes,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure_positive(self.max_concurrent_agents, "max_concurrent_agents")?;
        ensure_positive(self.max_agents_per_run, "max_agents_per_run")?;
        if self.max_concurrent_agents > self.max_agents_per_run {
            anyhow::bail!("dynamic_workflows.max_concurrent_agents must be <= max_agents_per_run");
        }
        ensure_positive(
            self.default_agent_timeout_seconds,
            "default_agent_timeout_seconds",
        )?;
        ensure_positive(
            self.default_run_timeout_seconds,
            "default_run_timeout_seconds",
        )?;
        ensure_positive(self.default_checkpoint_bytes, "default_checkpoint_bytes")?;
        ensure_positive(self.max_report_bytes, "max_report_bytes")?;
        ensure_path(&self.workspace_workflows_dir, "workspace_workflows_dir")?;
        ensure_path(&self.user_workflows_dir, "user_workflows_dir")?;
        if let Some(ttl) = self.approval.consent_ttl_seconds {
            ensure_positive(ttl, "approval.consent_ttl_seconds")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicWorkflowDirectories {
    pub user: PathBuf,
    pub workspace: PathBuf,
}

pub fn resolve_workflow_directories(
    config: Option<&DynamicWorkflowsConfig>,
    workspace: Option<&Path>,
) -> DynamicWorkflowDirectories {
    let defaults;
    let config = match config {
        Some(config) => config,
        None => {
            defaults = DynamicWorkflowsConfig::default();
            &defaults
        }
    };
    DynamicWorkflowDirectories {
        user: resolve_user_workflows_dir(config),
        workspace: resolve_workspace_workflows_dir(config, workspace),
    }
}

pub fn resolve_user_workflows_dir(config: &DynamicWorkflowsConfig) -> PathBuf {
    expand_tilde(&config.user_workflows_dir)
}

pub fn resolve_workspace_workflows_dir(
    config: &DynamicWorkflowsConfig,
    workspace: Option<&Path>,
) -> PathBuf {
    let path = expand_tilde(&config.workspace_workflows_dir);
    if path.is_absolute() {
        return path;
    }
    workspace
        .map(|workspace| workspace.join(&path))
        .unwrap_or(path)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DynamicWorkflowApprovalConfig {
    #[serde(default = "default_true")]
    pub require_approval: bool,
    #[serde(default = "default_true")]
    pub allow_always_for_script_and_workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_ttl_seconds: Option<u64>,
}

impl Default for DynamicWorkflowApprovalConfig {
    fn default() -> Self {
        Self {
            require_approval: true,
            allow_always_for_script_and_workspace: true,
            consent_ttl_seconds: None,
        }
    }
}

pub fn validate_config(config: &Config) -> anyhow::Result<()> {
    if let Some(dynamic_workflows) = &config.dynamic_workflows {
        dynamic_workflows.validate()?;
    }
    Ok(())
}

pub(crate) fn apply_env_overrides_with(
    config: &mut DynamicWorkflowsConfig,
    env: &mut impl FnMut(&str) -> Option<String>,
) {
    if let Some(disabled) = env("RODER_DYNAMIC_WORKFLOWS_DISABLED")
        && parse_bool(&disabled).unwrap_or(false)
    {
        config.enabled = false;
    }
    if let Some(enabled) = env("RODER_DYNAMIC_WORKFLOWS_ENABLED")
        && let Some(enabled) = parse_bool(&enabled)
    {
        config.enabled = enabled;
    }
    if let Some(trigger) = env("RODER_DYNAMIC_WORKFLOWS_TRIGGER_WORD")
        && let Some(trigger) = parse_bool(&trigger)
    {
        config.trigger_word_enabled = trigger;
    }
    if let Some(auto) = env("RODER_DYNAMIC_WORKFLOWS_AUTO_WITH_ULTRACODE")
        && let Some(auto) = parse_bool(&auto)
    {
        config.auto_with_ultracode = auto;
    }
    if let Some(max_agents) = env("RODER_DYNAMIC_WORKFLOWS_MAX_AGENTS")
        && let Ok(max_agents) = max_agents.trim().parse::<u32>()
    {
        config.max_agents_per_run = max_agents;
    }
    if let Some(max_concurrent) = env("RODER_DYNAMIC_WORKFLOWS_MAX_CONCURRENT_AGENTS")
        && let Ok(max_concurrent) = max_concurrent.trim().parse::<u32>()
    {
        config.max_concurrent_agents = max_concurrent;
    }
    if let Some(timeout) = env("RODER_DYNAMIC_WORKFLOWS_AGENT_TIMEOUT_SECONDS")
        && let Ok(timeout) = timeout.trim().parse::<u64>()
    {
        config.default_agent_timeout_seconds = timeout;
    }
    if let Some(timeout) = env("RODER_DYNAMIC_WORKFLOWS_RUN_TIMEOUT_SECONDS")
        && let Ok(timeout) = timeout.trim().parse::<u64>()
    {
        config.default_run_timeout_seconds = timeout;
    }
    if let Some(bytes) = env("RODER_DYNAMIC_WORKFLOWS_CHECKPOINT_BYTES")
        && let Ok(bytes) = bytes.trim().parse::<u64>()
    {
        config.default_checkpoint_bytes = bytes;
    }
    if let Some(bytes) = env("RODER_DYNAMIC_WORKFLOWS_MAX_REPORT_BYTES")
        && let Ok(bytes) = bytes.trim().parse::<u64>()
    {
        config.max_report_bytes = bytes;
    }
    if let Some(path) = env("RODER_DYNAMIC_WORKFLOWS_WORKSPACE_DIR")
        && !path.trim().is_empty()
    {
        config.workspace_workflows_dir = PathBuf::from(path);
    }
    if let Some(path) = env("RODER_DYNAMIC_WORKFLOWS_USER_DIR")
        && !path.trim().is_empty()
    {
        config.user_workflows_dir = PathBuf::from(path);
    }
    if let Some(require) = env("RODER_DYNAMIC_WORKFLOWS_REQUIRE_APPROVAL")
        && let Some(require) = parse_bool(&require)
    {
        config.approval.require_approval = require;
    }
    if let Some(allow) = env("RODER_DYNAMIC_WORKFLOWS_ALLOW_ALWAYS_APPROVAL")
        && let Some(allow) = parse_bool(&allow)
    {
        config.approval.allow_always_for_script_and_workspace = allow;
    }
    if let Some(ttl) = env("RODER_DYNAMIC_WORKFLOWS_CONSENT_TTL_SECONDS")
        && let Ok(ttl) = ttl.trim().parse::<u64>()
    {
        config.approval.consent_ttl_seconds = Some(ttl);
    }
    if let Some(live) = env("RODER_DYNAMIC_WORKFLOWS_LIVE") {
        config.live_checks = parse_bool(&live).unwrap_or(false);
    }
    if let Some(live) = env("RODER_DEEP_RESEARCH_LIVE") {
        config.deep_research_live = parse_bool(&live).unwrap_or(false);
    }
}

fn ensure_positive<T>(value: T, name: &str) -> anyhow::Result<()>
where
    T: PartialEq + From<u8> + std::fmt::Display,
{
    if value == T::from(0) {
        anyhow::bail!("dynamic_workflows.{name} must be greater than 0");
    }
    Ok(())
}

fn ensure_path(path: &Path, name: &str) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        anyhow::bail!("dynamic_workflows.{name} cannot be empty");
    }
    Ok(())
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        dirs::home_dir().unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent_agents() -> u32 {
    16
}

fn default_max_agents_per_run() -> u32 {
    1000
}

fn default_agent_timeout_seconds() -> u64 {
    180
}

fn default_run_timeout_seconds() -> u64 {
    14_400
}

fn default_checkpoint_bytes() -> u64 {
    1_048_576
}

fn default_max_report_bytes() -> u64 {
    65_536
}

fn default_workspace_workflows_dir() -> PathBuf {
    PathBuf::from(".agents/workflows")
}

fn default_user_workflows_dir() -> PathBuf {
    PathBuf::from("~/.roder/workflows")
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_workflows_config_parses_defaults_and_limits() {
        let config: Config = toml::from_str(
            r#"
            [dynamic_workflows]
            enabled = true
            trigger_word_enabled = true
            auto_with_ultracode = true
            max_concurrent_agents = 8
            max_agents_per_run = 32
            default_agent_timeout_seconds = 120
            default_run_timeout_seconds = 3600
            default_checkpoint_bytes = 131072
            max_report_bytes = 32768
            workspace_workflows_dir = ".agents/workflows"
            user_workflows_dir = "~/.roder/workflows"
            live_checks = false

            [dynamic_workflows.approval]
            require_approval = true
            allow_always_for_script_and_workspace = true
            consent_ttl_seconds = 86400
            "#,
        )
        .unwrap();

        let dynamic = config.dynamic_workflows.unwrap();
        assert_eq!(dynamic.max_concurrent_agents, 8);
        assert_eq!(dynamic.max_agents_per_run, 32);
        assert_eq!(dynamic.limits().max_report_bytes, 32_768);
        assert_eq!(dynamic.approval.consent_ttl_seconds, Some(86_400));
        dynamic.validate().unwrap();
    }

    #[test]
    fn dynamic_workflows_env_overrides_apply_without_mutating_process_env() {
        let mut config = DynamicWorkflowsConfig::default();
        apply_env_overrides_with(&mut config, &mut |key| match key {
            "RODER_DYNAMIC_WORKFLOWS_DISABLED" => Some("true".to_string()),
            "RODER_DYNAMIC_WORKFLOWS_MAX_AGENTS" => Some("24".to_string()),
            "RODER_DYNAMIC_WORKFLOWS_MAX_CONCURRENT_AGENTS" => Some("6".to_string()),
            "RODER_DYNAMIC_WORKFLOWS_WORKSPACE_DIR" => Some(".agents/wf".to_string()),
            "RODER_DYNAMIC_WORKFLOWS_CONSENT_TTL_SECONDS" => Some("30".to_string()),
            "RODER_DYNAMIC_WORKFLOWS_LIVE" => Some("1".to_string()),
            "RODER_DEEP_RESEARCH_LIVE" => Some("true".to_string()),
            _ => None,
        });

        assert!(!config.enabled);
        assert_eq!(config.max_agents_per_run, 24);
        assert_eq!(config.max_concurrent_agents, 6);
        assert_eq!(config.workspace_workflows_dir, PathBuf::from(".agents/wf"));
        assert_eq!(config.approval.consent_ttl_seconds, Some(30));
        assert!(config.live_checks);
        assert!(config.deep_research_live);
    }

    #[test]
    fn dynamic_workflows_validation_rejects_runaway_limits() {
        let config = DynamicWorkflowsConfig {
            max_concurrent_agents: 5,
            max_agents_per_run: 4,
            ..DynamicWorkflowsConfig::default()
        };

        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("max_concurrent_agents"));
    }

    #[test]
    fn dynamic_workflows_directory_resolution_uses_workspace_base() {
        let config = DynamicWorkflowsConfig {
            workspace_workflows_dir: PathBuf::from(".roder-workflows"),
            user_workflows_dir: PathBuf::from("/tmp/roder-user-workflows"),
            ..DynamicWorkflowsConfig::default()
        };

        let dirs = resolve_workflow_directories(Some(&config), Some(Path::new("/tmp/workspace")));

        assert_eq!(
            dirs.workspace,
            PathBuf::from("/tmp/workspace/.roder-workflows")
        );
        assert_eq!(dirs.user, PathBuf::from("/tmp/roder-user-workflows"));
    }
}
