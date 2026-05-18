use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub mod workflow_import;

pub use workflow_import::{WorkflowScanOptions, scan_workflow_imports};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub web_search: Option<WebSearchConfig>,
    pub subagents: Option<SubagentsConfig>,
    pub policy_modes: Option<PolicyModesConfig>,
    pub commands: Option<CommandsConfig>,
    pub notifications: Option<NotificationsConfig>,
    pub tui: Option<TuiConfig>,
    pub remote_runners: Option<RemoteRunnersConfig>,
    pub media: Option<MediaConfig>,
    pub memories: Option<MemoriesConfig>,
    #[serde(default)]
    pub embedding_providers: HashMap<String, EmbeddingProviderConfig>,
    pub agent_teams: Option<AgentTeamsConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelConfig {
    pub edit_tool: Option<String>,
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub enabled: bool,
    pub mode: Option<String>,
    pub provider: Option<String>,
    #[serde(default)]
    pub canonical_tool: bool,
    #[serde(default)]
    pub namespaced_tools: bool,
    pub max_results: Option<u8>,
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub firecrawl: WebSearchProviderConfig,
    #[serde(default)]
    pub perplexity: WebSearchProviderConfig,
    #[serde(default)]
    pub tavily: WebSearchProviderConfig,
    #[serde(default)]
    pub parallel: WebSearchProviderConfig,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: None,
            provider: None,
            canonical_tool: true,
            namespaced_tools: false,
            max_results: None,
            timeout_seconds: None,
            firecrawl: WebSearchProviderConfig::default(),
            perplexity: WebSearchProviderConfig::default(),
            tavily: WebSearchProviderConfig::default(),
            parallel: WebSearchProviderConfig::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchProviderConfig {
    #[serde(default)]
    pub enabled: bool,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub project: Option<String>,
    pub project_env: Option<String>,
    pub search_depth: Option<String>,
    pub mode: Option<String>,
    #[serde(default)]
    pub debug_raw_response: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentsConfig {
    #[serde(default)]
    pub enabled: bool,
    pub default_agent: Option<String>,
    pub default_timeout_seconds: Option<u64>,
    pub max_concurrent: Option<usize>,
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub include_child_transcript: bool,
    #[serde(default)]
    pub expose_per_type: bool,
    #[serde(default)]
    pub allow_extension_overrides: bool,
    #[serde(default)]
    pub live_test: bool,
    #[serde(default)]
    pub disk: SubagentsDiskConfig,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentsDiskConfig {
    pub user_dir: Option<PathBuf>,
    pub workspace_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyModesConfig {
    pub default: Option<String>,
    pub warn_on_bypass: Option<bool>,
    pub plan_blocks_network: Option<bool>,
    pub exit_plan_requires_summary: Option<bool>,
    #[serde(default)]
    pub auto_approve: PolicyAutoApproveConfig,
}

impl Default for PolicyModesConfig {
    fn default() -> Self {
        Self {
            default: Some("default".to_string()),
            warn_on_bypass: Some(true),
            plan_blocks_network: Some(false),
            exit_plan_requires_summary: Some(true),
            auto_approve: PolicyAutoApproveConfig::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyAutoApproveConfig {
    #[serde(default)]
    pub default: Vec<String>,
    #[serde(default, alias = "accept_edits")]
    pub accept_all: Vec<String>,
    #[serde(default)]
    pub plan: Vec<String>,
    #[serde(default)]
    pub bypass: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub user_dir: Option<PathBuf>,
    pub workspace_dir: Option<PathBuf>,
    #[serde(default)]
    pub allow_shell_includes: bool,
    #[serde(default)]
    pub allow_url_includes: bool,
    #[serde(default)]
    pub allowed_url_hosts: Vec<String>,
    pub include_timeout_seconds: Option<u64>,
    pub max_include_bytes: Option<usize>,
    #[serde(default = "default_true")]
    pub live_reload: bool,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_dir: None,
            workspace_dir: Some(PathBuf::from(".roder/commands")),
            allow_shell_includes: false,
            allow_url_includes: false,
            allowed_url_hosts: Vec::new(),
            include_timeout_seconds: Some(5),
            max_include_bytes: Some(65_536),
            live_reload: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub terminal: NotificationSinkConfig,
    #[serde(default)]
    pub desktop: NotificationSinkConfig,
    #[serde(default)]
    pub live_notifications: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            kinds: Vec::new(),
            terminal: NotificationSinkConfig { enabled: true },
            desktop: NotificationSinkConfig { enabled: true },
            live_notifications: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationSinkConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for NotificationSinkConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiConfig {
    #[serde(default)]
    pub status: TuiStatusConfig,
    #[serde(default)]
    pub palette: TuiPaletteConfig,
    #[serde(default)]
    pub diff: TuiDiffConfig,
    #[serde(default)]
    pub keymap: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiStatusConfig {
    #[serde(default)]
    pub disabled_segments: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiPaletteConfig {
    #[serde(default)]
    pub disabled_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiDiffConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRunnersConfig {
    #[serde(default)]
    pub enabled: bool,
    pub default_destination: Option<String>,
    #[serde(default)]
    pub destinations: HashMap<String, RemoteRunnerDestinationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaConfig {
    pub artifacts_dir: Option<PathBuf>,
    pub max_read_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoriesConfig {
    pub store_path: Option<PathBuf>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    #[serde(default = "default_true")]
    pub project_enabled: bool,
    #[serde(default)]
    pub global_enabled: bool,
    #[serde(default)]
    pub include_global_with_project: bool,
}

impl Default for MemoriesConfig {
    fn default() -> Self {
        Self {
            store_path: None,
            embedding_provider: Some("openai".to_string()),
            embedding_model: Some("text-embedding-3-large".to_string()),
            project_enabled: true,
            global_enabled: false,
            include_global_with_project: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingProviderConfig {
    #[serde(default)]
    pub enabled: bool,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub command: Option<Vec<String>>,
    pub dimensions: Option<usize>,
}

impl Default for EmbeddingProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            api_key_env: None,
            command: None,
            dimensions: None,
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            artifacts_dir: None,
            max_read_bytes: Some(10 * 1024 * 1024),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTeamsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub display_mode: roder_api::teams::AgentTeamDisplayMode,
    pub default_teammate_model: Option<String>,
    #[serde(default)]
    pub require_plan_approval: bool,
    pub max_teammates: Option<usize>,
    #[serde(default)]
    pub split_panes: AgentTeamsSplitPaneConfig,
}

impl Default for AgentTeamsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            display_mode: roder_api::teams::AgentTeamDisplayMode::Auto,
            default_teammate_model: Some("lead".to_string()),
            require_plan_approval: false,
            max_teammates: Some(5),
            split_panes: AgentTeamsSplitPaneConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentTeamsSplitPaneConfig {
    #[serde(default = "default_true")]
    pub reuse_existing_tmux_session: bool,
    pub tmux_command: Option<String>,
    pub iterm2_command: Option<String>,
}

impl Default for AgentTeamsSplitPaneConfig {
    fn default() -> Self {
        Self {
            reuse_existing_tmux_session: true,
            tmux_command: Some("tmux".to_string()),
            iterm2_command: Some("it2".to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRunnerDestinationConfig {
    pub provider: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub secret_env: HashMap<String, String>,
}

impl Default for TuiDiffConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_true() -> bool {
    true
}

pub fn load_config() -> anyhow::Result<Config> {
    let mut config = load_config_file()?;
    apply_env_overrides(&mut config);
    Ok(config)
}

pub fn save_default_provider_model(provider: &str, model: &str) -> anyhow::Result<()> {
    save_default_provider_model_to_path(config_path(), provider, model)
}

pub fn save_default_provider_model_reasoning(
    provider: &str,
    model: &str,
    reasoning: Option<&str>,
) -> anyhow::Result<()> {
    save_default_provider_model_reasoning_to_path(config_path(), provider, model, reasoning)
}

pub fn save_web_search_mode(mode: &str) -> anyhow::Result<()> {
    save_web_search_mode_to_path(config_path(), mode)
}

pub fn save_default_policy_mode(mode: &str) -> anyhow::Result<()> {
    save_default_policy_mode_to_path(config_path(), mode)
}

pub fn save_memory_embedding_provider(provider: &str, model: &str) -> anyhow::Result<()> {
    save_memory_embedding_provider_to_path(config_path(), provider, model)
}

pub fn save_provider_api_key(provider: &str, api_key: &str) -> anyhow::Result<()> {
    save_provider_api_key_to_path(config_path(), provider, api_key)
}

pub fn save_default_provider_model_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    model: &str,
) -> anyhow::Result<()> {
    save_default_provider_model_reasoning_to_path(path, provider, model, None)
}

pub fn save_default_provider_model_reasoning_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    model: &str,
    reasoning: Option<&str>,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.provider = Some(provider.to_string());
    config.model = Some(model.to_string());
    if let Some(reasoning) = reasoning {
        config.reasoning = Some(reasoning.to_string());
    }
    save_config_file_to_path(path, &config)
}

pub fn save_web_search_mode_to_path(path: impl AsRef<Path>, mode: &str) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.web_search.get_or_insert_with(Default::default).mode = Some(mode.to_string());
    save_config_file_to_path(path, &config)
}

pub fn save_default_policy_mode_to_path(path: impl AsRef<Path>, mode: &str) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config
        .policy_modes
        .get_or_insert_with(Default::default)
        .default = Some(mode.to_string());
    save_config_file_to_path(path, &config)
}

pub fn save_memory_embedding_provider_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    model: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    let memories = config.memories.get_or_insert_with(Default::default);
    memories.embedding_provider = Some(provider.to_string());
    memories.embedding_model = Some(model.to_string());
    save_config_file_to_path(path, &config)
}

pub fn save_provider_api_key_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    api_key: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config
        .providers
        .entry(provider.to_string())
        .or_default()
        .api_key = Some(api_key.to_string());
    save_config_file_to_path(path, &config)
}

fn load_config_file() -> anyhow::Result<Config> {
    load_config_file_from_path(config_path())
}

fn load_config_file_from_path(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let path = path.as_ref();
    if path.exists() {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    } else {
        Ok(Config::default())
    }
}

fn save_config_file_to_path(path: impl AsRef<Path>, config: &Config) -> anyhow::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(config)?;
    fs::write(path, contents)?;
    Ok(())
}

fn config_path() -> PathBuf {
    let mut config_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    config_path.push(".roder");
    config_path.push("config.toml");
    config_path
}

fn apply_env_overrides(config: &mut Config) {
    apply_env_overrides_with(config, |key| std::env::var(key).ok());
}

fn apply_env_overrides_with(config: &mut Config, mut env: impl FnMut(&str) -> Option<String>) {
    if let Some(provider) = env("RODER_PROVIDER")
        && !provider.trim().is_empty()
    {
        config.provider = Some(provider);
    }
    if let Some(model) = env("RODER_MODEL")
        && !model.trim().is_empty()
    {
        config.model = Some(model);
    }
    if let Some(reasoning) = env("RODER_REASONING")
        && !reasoning.trim().is_empty()
    {
        config.reasoning = Some(reasoning);
    }
    if let Some(limit) = env("RODER_AUTO_COMPACT_TOKEN_LIMIT")
        && let Ok(limit) = limit.trim().parse::<u32>()
    {
        config.auto_compact_token_limit = Some(limit);
    }
    if let Some(default_agent) =
        env("RODER_SUBAGENTS_DEFAULT").or_else(|| env("RODER_SUBAGENTS_DEFAULT_AGENT"))
        && !default_agent.trim().is_empty()
    {
        config
            .subagents
            .get_or_insert_with(Default::default)
            .default_agent = Some(default_agent);
    }
    if let Some(max_depth) = env("RODER_SUBAGENTS_MAX_DEPTH")
        && let Ok(max_depth) = max_depth.trim().parse::<usize>()
    {
        config
            .subagents
            .get_or_insert_with(Default::default)
            .max_depth = Some(max_depth);
    }
    if let Some(live_test) = env("RODER_LIVE_SUBAGENTS")
        && parse_bool(&live_test).unwrap_or(false)
    {
        config
            .subagents
            .get_or_insert_with(Default::default)
            .live_test = true;
    }
    if let Some(mode) = env("RODER_POLICY_MODE")
        && !mode.trim().is_empty()
    {
        config
            .policy_modes
            .get_or_insert_with(Default::default)
            .default = Some(mode);
    }
    if let Some(warn) = env("RODER_WARN_ON_BYPASS")
        && let Some(warn) = parse_bool(&warn)
    {
        config
            .policy_modes
            .get_or_insert_with(Default::default)
            .warn_on_bypass = Some(warn);
    }
    if let Some(mode) = env("RODER_WEB_SEARCH_MODE")
        && !mode.trim().is_empty()
    {
        config.web_search.get_or_insert_with(Default::default).mode = Some(mode);
    }
    if let Some(disabled) = env("RODER_COMMANDS_DISABLED")
        && parse_bool(&disabled).unwrap_or(false)
    {
        config.commands.get_or_insert_with(Default::default).enabled = false;
    }
    if let Some(allow_shell) = env("RODER_COMMANDS_ALLOW_SHELL")
        && let Some(allow_shell) = parse_bool(&allow_shell)
    {
        config
            .commands
            .get_or_insert_with(Default::default)
            .allow_shell_includes = allow_shell;
    }
    if let Some(allow_url) = env("RODER_COMMANDS_ALLOW_URL")
        && let Some(allow_url) = parse_bool(&allow_url)
    {
        config
            .commands
            .get_or_insert_with(Default::default)
            .allow_url_includes = allow_url;
    }
    if let Some(disabled) = env("RODER_NOTIFICATIONS_DISABLED")
        && parse_bool(&disabled).unwrap_or(false)
    {
        config
            .notifications
            .get_or_insert_with(Default::default)
            .enabled = false;
    }
    if let Some(terminal) = env("RODER_NOTIFY_TERMINAL")
        && let Some(terminal) = parse_bool(&terminal)
    {
        config
            .notifications
            .get_or_insert_with(Default::default)
            .terminal
            .enabled = terminal;
    }
    if let Some(desktop) = env("RODER_NOTIFY_DESKTOP")
        && let Some(desktop) = parse_bool(&desktop)
    {
        config
            .notifications
            .get_or_insert_with(Default::default)
            .desktop
            .enabled = desktop;
    }
    if let Some(kinds) = env("RODER_NOTIFY_KINDS") {
        let kinds = kinds
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !kinds.is_empty() {
            config
                .notifications
                .get_or_insert_with(Default::default)
                .kinds = kinds;
        }
    }
    if let Some(live) = env("RODER_LIVE_NOTIFICATIONS")
        && parse_bool(&live).unwrap_or(false)
    {
        config
            .notifications
            .get_or_insert_with(Default::default)
            .live_notifications = true;
    }
    if let Some(destination) = env("RODER_REMOTE_RUNNER")
        && !destination.trim().is_empty()
    {
        let remote = config.remote_runners.get_or_insert_with(Default::default);
        remote.enabled = true;
        remote.default_destination = Some(destination);
    }
    if let Some(path) = env("RODER_MEMORIES_PATH")
        && !path.trim().is_empty()
    {
        config
            .memories
            .get_or_insert_with(Default::default)
            .store_path = Some(PathBuf::from(path));
    }
    if let Some(provider) = env("RODER_MEMORY_EMBEDDING_PROVIDER")
        && !provider.trim().is_empty()
    {
        config
            .memories
            .get_or_insert_with(Default::default)
            .embedding_provider = Some(provider);
    }
    if let Some(model) = env("RODER_MEMORY_EMBEDDING_MODEL")
        && !model.trim().is_empty()
    {
        config
            .memories
            .get_or_insert_with(Default::default)
            .embedding_model = Some(model);
    }
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
    fn serializes_provider_and_model_without_dropping_provider_blocks() {
        let mut config = Config {
            provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("medium".to_string()),
            auto_compact_token_limit: None,
            web_search: None,
            subagents: None,
            policy_modes: None,
            commands: None,
            notifications: None,
            tui: None,
            remote_runners: None,
            media: None,
            memories: None,
            embedding_providers: HashMap::new(),
            agent_teams: None,
            providers: HashMap::new(),
            models: HashMap::new(),
        };
        config.providers.insert(
            "openai".to_string(),
            ProviderConfig {
                api_key: Some("key".to_string()),
            },
        );

        let encoded = toml::to_string_pretty(&config).unwrap();
        assert!(encoded.contains("provider = \"codex\""));
        assert!(encoded.contains("model = \"gpt-5.5\""));
        assert!(encoded.contains("[providers.openai]"));
        assert!(encoded.contains("api_key = \"key\""));
    }

    #[test]
    fn memories_embeddings_config_deserializes_and_env_overrides_apply() {
        let mut config: Config = toml::from_str(
            r#"
            [memories]
            store_path = "/tmp/roder-mem.sqlite3"
            embedding_provider = "openai"
            embedding_model = "text-embedding-3-large"
            include_global_with_project = true

            [embedding_providers.local]
            enabled = true
            command = ["embedder", "--json"]
            dimensions = 384
            "#,
        )
        .unwrap();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_MEMORY_EMBEDDING_PROVIDER" => Some("local".to_string()),
            "RODER_MEMORY_EMBEDDING_MODEL" => Some("mini".to_string()),
            _ => None,
        });
        let memories = config.memories.unwrap();
        assert_eq!(memories.embedding_provider.as_deref(), Some("local"));
        assert_eq!(memories.embedding_model.as_deref(), Some("mini"));
        assert_eq!(
            config.embedding_providers["local"]
                .command
                .as_ref()
                .unwrap()[0],
            "embedder"
        );
    }

    #[test]
    fn deserializes_subagents_config() {
        let config: Config = toml::from_str(
            r#"
            [subagents]
            enabled = true
            default_agent = "explore"
            default_timeout_seconds = 90
            max_concurrent = 3
            max_depth = 2
            include_child_transcript = true
            expose_per_type = true

            [subagents.disk]
            user_dir = "~/.roder/agents"
            workspace_dir = ".roder/agents"
            "#,
        )
        .unwrap();

        let subagents = config.subagents.unwrap();
        assert!(subagents.enabled);
        assert_eq!(subagents.default_agent.as_deref(), Some("explore"));
        assert_eq!(subagents.default_timeout_seconds, Some(90));
        assert_eq!(subagents.max_concurrent, Some(3));
        assert_eq!(subagents.max_depth, Some(2));
        assert!(subagents.include_child_transcript);
        assert!(subagents.expose_per_type);
        assert_eq!(
            subagents.disk.workspace_dir.as_deref(),
            Some(Path::new(".roder/agents"))
        );
    }

    #[test]
    fn subagent_env_overrides_apply_without_mutating_process_env() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_SUBAGENTS_DEFAULT" => Some("review".to_string()),
            "RODER_SUBAGENTS_MAX_DEPTH" => Some("4".to_string()),
            "RODER_LIVE_SUBAGENTS" => Some("true".to_string()),
            _ => None,
        });

        let subagents = config.subagents.unwrap();
        assert_eq!(subagents.default_agent.as_deref(), Some("review"));
        assert_eq!(subagents.max_depth, Some(4));
        assert!(subagents.live_test);
    }

    #[test]
    fn deserializes_policy_modes_config() {
        let config: Config = toml::from_str(
            r#"
            [policy_modes]
            default = "plan"
            warn_on_bypass = false
            plan_blocks_network = true
            exit_plan_requires_summary = true

            [policy_modes.auto_approve]
            accept_all = ["fs.write", "fs.edit"]
            bypass = ["*"]
            "#,
        )
        .unwrap();

        let policy_modes = config.policy_modes.unwrap();
        assert_eq!(policy_modes.default.as_deref(), Some("plan"));
        assert_eq!(policy_modes.warn_on_bypass, Some(false));
        assert_eq!(policy_modes.plan_blocks_network, Some(true));
        assert_eq!(
            policy_modes.auto_approve.accept_all,
            vec!["fs.write".to_string(), "fs.edit".to_string()]
        );
    }

    #[test]
    fn deserializes_legacy_accept_edits_auto_approve_config() {
        let config: Config = toml::from_str(
            r#"
            [policy_modes.auto_approve]
            accept_edits = ["shell"]
            "#,
        )
        .unwrap();

        assert_eq!(
            config.policy_modes.unwrap().auto_approve.accept_all,
            vec!["shell".to_string()]
        );
    }

    #[test]
    fn policy_mode_env_overrides_apply_without_mutating_process_env() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_POLICY_MODE" => Some("bypass".to_string()),
            "RODER_WARN_ON_BYPASS" => Some("false".to_string()),
            _ => None,
        });

        let policy_modes = config.policy_modes.unwrap();
        assert_eq!(policy_modes.default.as_deref(), Some("bypass"));
        assert_eq!(policy_modes.warn_on_bypass, Some(false));
    }

    #[test]
    fn deserializes_commands_config_and_env_overrides() {
        let config: Config = toml::from_str(
            r#"
            [commands]
            enabled = true
            user_dir = "~/.roder/commands"
            workspace_dir = ".roder/commands"
            allow_shell_includes = false
            allow_url_includes = false
            allowed_url_hosts = ["example.com"]
            include_timeout_seconds = 7
            max_include_bytes = 4096
            live_reload = true
            "#,
        )
        .unwrap();

        let commands = config.commands.unwrap();
        assert!(commands.enabled);
        assert_eq!(
            commands.workspace_dir.as_deref(),
            Some(Path::new(".roder/commands"))
        );
        assert_eq!(commands.allowed_url_hosts, vec!["example.com".to_string()]);
        assert_eq!(commands.include_timeout_seconds, Some(7));

        let mut config = Config::default();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_COMMANDS_DISABLED" => Some("true".to_string()),
            "RODER_COMMANDS_ALLOW_SHELL" => Some("true".to_string()),
            "RODER_COMMANDS_ALLOW_URL" => Some("true".to_string()),
            _ => None,
        });
        let commands = config.commands.unwrap();
        assert!(!commands.enabled);
        assert!(commands.allow_shell_includes);
        assert!(commands.allow_url_includes);
    }

    #[test]
    fn deserializes_notifications_config_and_env_overrides() {
        let config: Config = toml::from_str(
            r#"
            [notifications]
            enabled = true
            kinds = ["needs_input", "task_failed"]
            live_notifications = true

            [notifications.terminal]
            enabled = false

            [notifications.desktop]
            enabled = true
            "#,
        )
        .unwrap();

        let notifications = config.notifications.unwrap();
        assert!(notifications.enabled);
        assert_eq!(
            notifications.kinds,
            vec!["needs_input".to_string(), "task_failed".to_string()]
        );
        assert!(!notifications.terminal.enabled);
        assert!(notifications.desktop.enabled);
        assert!(notifications.live_notifications);

        let mut config = Config::default();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_NOTIFICATIONS_DISABLED" => Some("true".to_string()),
            "RODER_NOTIFY_TERMINAL" => Some("false".to_string()),
            "RODER_NOTIFY_DESKTOP" => Some("true".to_string()),
            "RODER_NOTIFY_KINDS" => Some("needs_input,task_completed".to_string()),
            "RODER_LIVE_NOTIFICATIONS" => Some("true".to_string()),
            _ => None,
        });
        let notifications = config.notifications.unwrap();
        assert!(!notifications.enabled);
        assert!(!notifications.terminal.enabled);
        assert!(notifications.desktop.enabled);
        assert_eq!(
            notifications.kinds,
            vec!["needs_input".to_string(), "task_completed".to_string()]
        );
        assert!(notifications.live_notifications);
    }

    #[test]
    fn deserializes_tui_keymap_config() {
        let config: Config = toml::from_str(
            r#"
            [tui.keymap]
            "palette/open" = ["ctrl+k"]
            "selection/copy" = ["y"]
            "#,
        )
        .unwrap();

        let keymap = config.tui.unwrap().keymap;
        assert_eq!(
            keymap.get("palette/open"),
            Some(&vec!["ctrl+k".to_string()])
        );
        assert_eq!(keymap.get("selection/copy"), Some(&vec!["y".to_string()]));
    }

    #[test]
    fn deserializes_agent_teams_config() {
        let config: Config = toml::from_str(
            r#"
            [agent_teams]
            enabled = true
            display_mode = "in_process"
            default_teammate_model = "lead"
            require_plan_approval = true
            max_teammates = 4

            [agent_teams.split_panes]
            reuse_existing_tmux_session = false
            tmux_command = "tmux-custom"
            iterm2_command = "it2-custom"
            "#,
        )
        .unwrap();

        let teams = config.agent_teams.unwrap();
        assert!(teams.enabled);
        assert_eq!(
            teams.display_mode,
            roder_api::teams::AgentTeamDisplayMode::InProcess
        );
        assert_eq!(teams.default_teammate_model.as_deref(), Some("lead"));
        assert!(teams.require_plan_approval);
        assert_eq!(teams.max_teammates, Some(4));
        assert!(!teams.split_panes.reuse_existing_tmux_session);
        assert_eq!(
            teams.split_panes.tmux_command.as_deref(),
            Some("tmux-custom")
        );
        assert_eq!(
            teams.split_panes.iterm2_command.as_deref(),
            Some("it2-custom")
        );
    }

    #[test]
    fn remote_runner_config_deserializes_and_serializes_secret_env_refs() {
        let config: Config = toml::from_str(
            r#"
            [remote_runners]
            enabled = true
            default_destination = "docker-dev"

            [remote_runners.destinations.docker-dev]
            provider = "docker"
            config = { image = "rust:latest" }
            secret_env = { DOCKER_TOKEN = "RODER_DOCKER_TOKEN" }
            "#,
        )
        .unwrap();

        let remote = config.remote_runners.unwrap();
        assert!(remote.enabled);
        assert_eq!(remote.default_destination.as_deref(), Some("docker-dev"));
        let destination = remote.destinations.get("docker-dev").unwrap();
        assert_eq!(destination.provider, "docker");
        assert_eq!(
            destination
                .secret_env
                .get("DOCKER_TOKEN")
                .map(String::as_str),
            Some("RODER_DOCKER_TOKEN")
        );

        let encoded = toml::to_string(&remote).unwrap();
        assert!(encoded.contains("RODER_DOCKER_TOKEN"));
        assert!(!encoded.contains("actual-secret-value"));
    }

    #[test]
    fn remote_runner_env_override_selects_destination() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_REMOTE_RUNNER" => Some("unix-local".to_string()),
            _ => None,
        });

        let remote = config.remote_runners.unwrap();
        assert!(remote.enabled);
        assert_eq!(remote.default_destination.as_deref(), Some("unix-local"));
    }

    #[test]
    fn deserializes_web_search_config() {
        let config: Config = toml::from_str(
            r#"
            [web_search]
            enabled = true
            mode = "external"
            provider = "tavily"
            namespaced_tools = true
            max_results = 8
            timeout_seconds = 20

            [web_search.tavily]
            enabled = true
            api_key_env = "TAVILY_API_KEY"
            project_env = "TAVILY_PROJECT"
            base_url = "https://api.tavily.com"
            search_depth = "basic"
            "#,
        )
        .unwrap();

        let web_search = config.web_search.unwrap();
        assert!(web_search.enabled);
        assert_eq!(web_search.mode.as_deref(), Some("external"));
        assert_eq!(web_search.provider.as_deref(), Some("tavily"));
        assert!(web_search.namespaced_tools);
        assert_eq!(web_search.max_results, Some(8));
        assert_eq!(
            web_search.tavily.api_key_env.as_deref(),
            Some("TAVILY_API_KEY")
        );
        assert_eq!(
            web_search.tavily.project_env.as_deref(),
            Some("TAVILY_PROJECT")
        );
    }

    #[test]
    fn web_search_mode_env_override_applies_without_mutating_process_env() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_WEB_SEARCH_MODE" => Some("live".to_string()),
            _ => None,
        });

        assert_eq!(config.web_search.unwrap().mode.as_deref(), Some("live"));
    }

    #[test]
    fn deserializes_custom_model_edit_tool_config() {
        let config: Config = toml::from_str(
            r#"
            [models."custom-openai"]
            edit_tool = "patch"
            parallel_tool_calls = false

            [models."custom-claude"]
            edit_tool = "edit"
            parallel_tool_calls = true
            "#,
        )
        .unwrap();

        assert_eq!(
            config
                .models
                .get("custom-openai")
                .and_then(|model| model.edit_tool.as_deref()),
            Some("patch")
        );
        assert_eq!(
            config
                .models
                .get("custom-openai")
                .and_then(|model| model.parallel_tool_calls),
            Some(false)
        );
        assert_eq!(
            config
                .models
                .get("custom-claude")
                .and_then(|model| model.edit_tool.as_deref()),
            Some("edit")
        );
        assert_eq!(
            config
                .models
                .get("custom-claude")
                .and_then(|model| model.parallel_tool_calls),
            Some(true)
        );
    }

    #[test]
    fn save_default_provider_model_creates_parent_directory() {
        let path = std::env::temp_dir()
            .join(format!("roder-config-test-{}", std::process::id()))
            .join("nested")
            .join("config.toml");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

        save_default_provider_model_to_path(&path, "codex", "gpt-5.5").unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains("model = \"gpt-5.5\""));

        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn save_web_search_mode_creates_or_updates_web_search_config() {
        let path = std::env::temp_dir().join(format!(
            "roder-config-web-search-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        save_web_search_mode_to_path(&path, "live").unwrap();

        let config = load_config_file_from_path(&path).unwrap();
        assert_eq!(config.web_search.unwrap().mode.as_deref(), Some("live"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_default_policy_mode_creates_or_updates_policy_modes_config() {
        let path = std::env::temp_dir().join(format!(
            "roder-config-policy-mode-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        save_default_policy_mode_to_path(&path, "accept_edits").unwrap();

        let config = load_config_file_from_path(&path).unwrap();
        assert_eq!(
            config
                .policy_modes
                .and_then(|policy| policy.default)
                .as_deref(),
            Some("accept_edits")
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_provider_api_key_creates_provider_config() {
        let path = std::env::temp_dir().join(format!(
            "roder-config-provider-key-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        save_provider_api_key_to_path(&path, "opencode", "sk-test").unwrap();

        let config = load_config_file_from_path(&path).unwrap();
        assert_eq!(
            config
                .providers
                .get("opencode")
                .and_then(|provider| provider.api_key.as_deref()),
            Some("sk-test")
        );
        let _ = fs::remove_file(&path);
    }
}
