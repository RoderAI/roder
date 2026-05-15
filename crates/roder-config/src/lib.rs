use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub web_search: Option<WebSearchConfig>,
    pub commands: Option<CommandsConfig>,
    pub subagents: Option<SubagentsConfig>,
    pub policy_modes: Option<PolicyModesConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
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
            workspace_dir: None,
            allow_shell_includes: false,
            allow_url_includes: false,
            include_timeout_seconds: Some(5),
            max_include_bytes: Some(65_536),
            live_reload: true,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub enabled: bool,
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
    #[serde(default)]
    pub accept_edits: Vec<String>,
    #[serde(default)]
    pub plan: Vec<String>,
    #[serde(default)]
    pub bypass: Vec<String>,
}

pub fn load_config() -> anyhow::Result<Config> {
    let mut config = load_config_file()?;
    apply_env_overrides(&mut config);
    Ok(config)
}

pub fn save_default_provider_model(provider: &str, model: &str) -> anyhow::Result<()> {
    save_default_provider_model_to_path(config_path(), provider, model)
}

pub fn save_default_provider_model_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    model: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.provider = Some(provider.to_string());
    config.model = Some(model.to_string());
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
}

fn default_true() -> bool {
    true
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
            commands: None,
            subagents: None,
            policy_modes: None,
            providers: HashMap::new(),
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
    fn deserializes_commands_config() {
        let config: Config = toml::from_str(
            r#"
            [commands]
            enabled = true
            user_dir = "~/.roder/commands"
            workspace_dir = ".roder/commands"
            allow_shell_includes = true
            allow_url_includes = true
            include_timeout_seconds = 9
            max_include_bytes = 1234
            live_reload = false
            "#,
        )
        .unwrap();

        let commands = config.commands.unwrap();
        assert!(commands.enabled);
        assert_eq!(
            commands.user_dir.as_deref(),
            Some(Path::new("~/.roder/commands"))
        );
        assert_eq!(
            commands.workspace_dir.as_deref(),
            Some(Path::new(".roder/commands"))
        );
        assert!(commands.allow_shell_includes);
        assert!(commands.allow_url_includes);
        assert_eq!(commands.include_timeout_seconds, Some(9));
        assert_eq!(commands.max_include_bytes, Some(1234));
        assert!(!commands.live_reload);
    }

    #[test]
    fn command_env_overrides_apply_without_mutating_process_env() {
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
            accept_edits = ["fs.write", "fs.edit"]
            bypass = ["*"]
            "#,
        )
        .unwrap();

        let policy_modes = config.policy_modes.unwrap();
        assert_eq!(policy_modes.default.as_deref(), Some("plan"));
        assert_eq!(policy_modes.warn_on_bypass, Some(false));
        assert_eq!(policy_modes.plan_blocks_network, Some(true));
        assert_eq!(
            policy_modes.auto_approve.accept_edits,
            vec!["fs.write".to_string(), "fs.edit".to_string()]
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
    fn deserializes_web_search_config() {
        let config: Config = toml::from_str(
            r#"
            [web_search]
            enabled = true
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
}
