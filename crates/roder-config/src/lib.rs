use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub mod agent_node;
pub mod analytics;
pub mod dynamic_workflows;
pub mod hosted;
pub mod marketplaces;
pub mod packages;
pub mod workflow_import;

pub use dynamic_workflows::{DynamicWorkflowApprovalConfig, DynamicWorkflowsConfig};
pub use marketplaces::*;
pub use workflow_import::{WorkflowScanOptions, scan_workflow_imports};

pub const RODER_CONFIG_DIR_ENV: &str = "RODER_CONFIG_DIR";
pub const RODER_DATA_DIR_ENV: &str = "RODER_DATA_DIR";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub runtime_profile: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub reliability: Option<ReliabilityConfig>,
    pub speed_policy: Option<SpeedPolicyConfig>,
    pub inference_router: Option<InferenceRouterConfig>,
    pub web_search: Option<WebSearchConfig>,
    pub tool_search: Option<ToolSearchConfig>,
    pub dynamic_workflows: Option<DynamicWorkflowsConfig>,
    pub context: Option<ContextConfig>,
    pub sessions: Option<SessionsConfig>,
    pub subagents: Option<SubagentsConfig>,
    pub policy_modes: Option<PolicyModesConfig>,
    pub commands: Option<CommandsConfig>,
    pub tools: Option<ToolsConfig>,
    pub search_index: Option<SearchIndexConfig>,
    pub notifications: Option<NotificationsConfig>,
    pub tui: Option<TuiConfig>,
    pub app_server: Option<AppServerConfig>,
    pub remote_runners: Option<RemoteRunnersConfig>,
    pub zerolang: Option<ZerolangConfig>,
    pub media: Option<MediaConfig>,
    pub memories: Option<MemoriesConfig>,
    /// Project knowledge base (`[knowledge]`); markdown engine by default.
    pub knowledge: Option<KnowledgeConfig>,
    #[serde(default)]
    pub embedding_providers: HashMap<String, EmbeddingProviderConfig>,
    pub agent_teams: Option<AgentTeamsConfig>,
    pub skills: Option<roder_skills::SkillsConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
    #[serde(default)]
    pub model_profiles: HashMap<String, ModelHarnessProfileConfig>,
    /// Process-hosted extensions (`[[process_extensions]]`), installed
    /// through `roder-ext-process-host` as ordinary registry extensions.
    #[serde(default)]
    pub process_extensions: Vec<roder_api::process_extension::ProcessExtensionConfig>,
    /// Package install settings (`[packages]`), e.g. the npm wrapper command.
    pub packages: Option<packages::PackagesConfig>,
    /// Local usage analytics (`[analytics]`); local-only, enabled by default.
    pub analytics: Option<analytics::AnalyticsConfig>,
    /// Workspace fork providers (`[forks]`).
    pub forks: Option<ForksConfig>,
    /// Remote agent-node connection profiles (`[[agent_nodes]]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_nodes: Vec<agent_node::AgentNodeProfile>,
}

/// `[forks]` config block (roadmap phase 81).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForksConfig {
    /// Default fork provider id (`git-worktree` when absent). Overridable
    /// per-invocation and via `RODER_FORK_PROVIDER`.
    #[serde(default)]
    pub default_provider: Option<String>,
    /// Base directory override for created fork workspaces (providers fall
    /// back to their own defaults, e.g. `<repo>/.roder/worktrees`).
    /// Overridable via `RODER_FORK_BASE_DIR`.
    #[serde(default)]
    pub base_dir: Option<String>,
}

/// Env override for the default fork provider.
pub const RODER_FORK_PROVIDER_ENV: &str = "RODER_FORK_PROVIDER";
/// Env override for the fork base directory.
pub const RODER_FORK_BASE_DIR_ENV: &str = "RODER_FORK_BASE_DIR";

/// Resolves the effective default fork provider: env override, then
/// `[forks].default_provider`, then `git-worktree`.
pub fn default_fork_provider(config: &Config) -> String {
    std::env::var(RODER_FORK_PROVIDER_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            config
                .forks
                .as_ref()
                .and_then(|forks| forks.default_provider.clone())
        })
        .unwrap_or_else(|| "git-worktree".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReliabilityConfig {
    pub max_consecutive_tool_failures: Option<u32>,
    pub max_tool_failures_per_turn: Option<u32>,
    pub max_model_calls_per_turn: Option<u32>,
    pub provider_retry_max_attempts: Option<u32>,
    pub provider_retry_initial_backoff_ms: Option<u64>,
    pub provider_retry_backoff_factor: Option<u32>,
    #[serde(default)]
    pub provider_retry_status_codes: Vec<u16>,
    pub retry_empty_provider_body: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpeedPolicyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub orientation_reasoning: Option<String>,
    pub execution_reasoning: Option<String>,
    pub verification_reasoning: Option<String>,
    pub recovery_reasoning: Option<String>,
    pub eval_deadline_seconds: Option<u64>,
}

impl Default for SpeedPolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            orientation_reasoning: None,
            execution_reasoning: None,
            verification_reasoning: None,
            recovery_reasoning: None,
            eval_deadline_seconds: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InferenceRouterConfig {
    #[serde(default)]
    pub enabled: bool,
    pub router: Option<String>,
    pub profile: Option<String>,
    pub baseline_provider: Option<String>,
    pub baseline_model: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extension: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionsConfig {
    #[serde(default = "default_session_store")]
    pub store: String,
    pub postgres: Option<PostgresSessionConfig>,
    pub mysql: Option<MysqlSessionConfig>,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            store: default_session_store(),
            postgres: None,
            mysql: None,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostgresSessionConfig {
    pub database_url: Option<String>,
    pub database_url_env: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_env: Option<String>,
    pub max_connections: Option<u32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MysqlSessionConfig {
    pub database_url: Option<String>,
    pub database_url_env: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_env: Option<String>,
    pub max_connections: Option<u32>,
}

fn default_session_store() -> String {
    "jsonl".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextConfig {
    #[serde(default = "default_true")]
    pub file_backed_dynamic_context: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            file_backed_dynamic_context: true,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub project_id: Option<String>,
    pub project_id_env: Option<String>,
    pub http_referer: Option<String>,
    pub app_title: Option<String>,
    pub cli_path: Option<String>,
    pub permission_mode: Option<String>,
    pub setting_sources: Option<Vec<String>>,
    pub tool_search: Option<ToolSearchConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelConfig {
    pub edit_tool: Option<String>,
    pub parallel_tool_calls: Option<bool>,
    pub tool_search: Option<ToolSearchConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSearchConfig {
    pub mode: Option<String>,
    pub max_catalog_items: Option<u32>,
    pub include_mcp: Option<bool>,
    pub include_skills: Option<bool>,
    pub fallback_to_explicit_tools: Option<bool>,
    pub provider_variant: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelHarnessProfileConfig {
    pub provider_family: Option<String>,
    pub edit_tool: Option<String>,
    pub schema_policy: Option<String>,
    pub instruction_overlay: Option<String>,
    pub parallel_tool_calls: Option<bool>,
    pub auto_compact_token_limit: Option<u32>,
    pub reasoning: Option<ModelProfileReasoningConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelProfileReasoningConfig {
    pub orientation: Option<String>,
    pub execution: Option<String>,
    pub verification: Option<String>,
    pub recovery: Option<String>,
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
pub struct ToolsConfig {
    #[serde(default = "default_tool_path_scope")]
    pub path_scope: String,
    pub shell: Option<String>,
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            path_scope: default_tool_path_scope(),
            shell: None,
            allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchIndexConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub max_file_bytes: Option<u64>,
    #[serde(default)]
    pub ignored_globs: Vec<String>,
    pub rebuild_concurrency: Option<usize>,
    pub max_index_bytes: Option<u64>,
}

impl Default for SearchIndexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_file_bytes: Some(1_048_576),
            ignored_globs: Vec::new(),
            rebuild_concurrency: Some(4),
            max_index_bytes: Some(512 * 1_048_576),
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
pub struct AppServerConfig {
    #[serde(default)]
    pub automations: AppServerAutomationsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppServerAutomationsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_automation_server_id")]
    pub server_id: String,
    #[serde(default = "default_automation_server_role")]
    pub server_role: String,
    #[serde(default = "default_automation_store_path")]
    pub store_path: PathBuf,
    #[serde(default = "default_automation_tick_seconds")]
    pub tick_seconds: u64,
    #[serde(default = "default_automation_lease_seconds")]
    pub lease_seconds: u64,
    #[serde(default = "default_automation_max_due_per_tick")]
    pub max_due_per_tick: u32,
    #[serde(default = "default_true")]
    pub run_missed_on_startup: bool,
    #[serde(default = "default_true")]
    pub read_api_when_disabled: bool,
    #[serde(default)]
    pub allowed_project_roots: Vec<PathBuf>,
}

impl Default for AppServerAutomationsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_id: default_automation_server_id(),
            server_role: default_automation_server_role(),
            store_path: default_automation_store_path(),
            tick_seconds: default_automation_tick_seconds(),
            lease_seconds: default_automation_lease_seconds(),
            max_due_per_tick: default_automation_max_due_per_tick(),
            run_missed_on_startup: true,
            read_api_when_disabled: true,
            allowed_project_roots: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRunnersConfig {
    #[serde(default)]
    pub enabled: bool,
    pub default_destination: Option<String>,
    #[serde(default)]
    pub destinations: HashMap<String, RemoteRunnerDestinationConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ZerolangConfig {
    pub binary: Option<PathBuf>,
    pub timeout_seconds: Option<u64>,
    pub artifact_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaConfig {
    pub artifacts_dir: Option<PathBuf>,
    pub max_read_bytes: Option<u64>,
    pub image_generation: Option<ImageGenerationConfig>,
}

/// `[media.image_generation]` settings for first-party image providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageGenerationConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub max_outputs: Option<u32>,
    pub max_input_images: Option<u32>,
    #[serde(default)]
    pub providers: BTreeMap<String, ImageProviderConfig>,
}

/// `[media.image_generation.providers.<id>]` overrides for one provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageProviderConfig {
    pub enabled: Option<bool>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

pub fn validate_media_config(config: &Config) -> anyhow::Result<()> {
    let Some(image_generation) = config
        .media
        .as_ref()
        .and_then(|media| media.image_generation.as_ref())
    else {
        return Ok(());
    };
    if let Some(max_outputs) = image_generation.max_outputs
        && max_outputs == 0
    {
        anyhow::bail!("media.image_generation.max_outputs must be at least 1");
    }
    if let Some(provider) = image_generation.default_provider.as_deref()
        && provider.trim().is_empty()
    {
        anyhow::bail!("media.image_generation.default_provider must not be empty");
    }
    for (provider_id, provider) in &image_generation.providers {
        if provider_id.trim().is_empty() {
            anyhow::bail!("media.image_generation.providers keys must not be empty");
        }
        if let Some(api_key_env) = provider.api_key_env.as_deref()
            && api_key_env.trim().is_empty()
        {
            anyhow::bail!(
                "media.image_generation.providers.{provider_id}.api_key_env must not be empty"
            );
        }
        // Validate model ids for the built-in catalog providers; custom
        // providers may register models Roder does not know about.
        if let Some(model) = provider.default_model.as_deref()
            && roder_api::catalog::lookup_image_provider(provider_id).is_some()
            && roder_api::catalog::lookup_image_model(provider_id, model).is_none()
        {
            anyhow::bail!(
                "media.image_generation.providers.{provider_id}.default_model {model:?} is not a known image model; known models: {}",
                roder_api::catalog::image_models_for_provider(provider_id)
                    .iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if let (Some(provider_id), Some(model)) = (
        image_generation.default_provider.as_deref(),
        image_generation.default_model.as_deref(),
    ) && roder_api::catalog::lookup_image_provider(provider_id).is_some()
        && roder_api::catalog::lookup_image_model(provider_id, model).is_none()
    {
        anyhow::bail!(
            "media.image_generation.default_model {model:?} is not a known {provider_id} image model"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoriesConfig {
    /// Memory store backend: "sqlite" (default) or "honcho".
    pub backend: Option<String>,
    pub store_path: Option<PathBuf>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    #[serde(default = "default_true")]
    pub project_enabled: bool,
    #[serde(default)]
    pub global_enabled: bool,
    #[serde(default)]
    pub include_global_with_project: bool,
    pub honcho: Option<HonchoMemoriesConfig>,
}

impl Default for MemoriesConfig {
    fn default() -> Self {
        Self {
            backend: None,
            store_path: None,
            embedding_provider: Some("openai".to_string()),
            embedding_model: Some("text-embedding-3-large".to_string()),
            project_enabled: true,
            global_enabled: false,
            include_global_with_project: false,
            honcho: None,
        }
    }
}

/// `[knowledge]` config block (roadmap phase 93). The markdown engine
/// (`roder-ext-knowledge-md`) is the only backend today; future engines
/// select through `backend`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeConfig {
    /// Knowledge store backend: "markdown" (default).
    pub backend: Option<String>,
    /// Store base path; defaults to `<roder-home>/knowledge`.
    pub store_path: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Inject relevant knowledge into turns.
    #[serde(default = "default_true")]
    pub recall: bool,
    /// Documents injected per turn at most.
    pub recall_limit: Option<usize>,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            backend: None,
            store_path: None,
            enabled: true,
            recall: true,
            recall_limit: None,
        }
    }
}

/// Settings for the Honcho memory backend. The api key itself is never
/// stored in config; `api_key_env` only names the environment variable that
/// holds it.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HonchoMemoriesConfig {
    pub base_url: Option<String>,
    pub workspace_id: Option<String>,
    pub peer_id: Option<String>,
    pub session_id: Option<String>,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingProviderConfig {
    #[serde(default)]
    pub enabled: bool,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub endpoint: Option<String>,
    pub command: Option<Vec<String>>,
    pub dimensions: Option<usize>,
    pub encoding_format: Option<String>,
    pub latency: Option<String>,
}

impl Default for EmbeddingProviderConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            api_key_env: None,
            endpoint: None,
            command: None,
            dimensions: None,
            encoding_format: None,
            latency: None,
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            artifacts_dir: None,
            max_read_bytes: Some(10 * 1024 * 1024),
            image_generation: None,
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

fn default_tool_path_scope() -> String {
    "global".to_string()
}

fn default_automation_server_id() -> String {
    "desktop-main".to_string()
}

fn default_automation_server_role() -> String {
    "desktop".to_string()
}

fn default_automation_store_path() -> PathBuf {
    PathBuf::from("~/.roder/automations.sqlite3")
}

fn default_automation_tick_seconds() -> u64 {
    30
}

fn default_automation_lease_seconds() -> u64 {
    900
}

fn default_automation_max_due_per_tick() -> u32 {
    10
}

pub fn load_config() -> anyhow::Result<Config> {
    let mut config = load_config_file()?;
    apply_env_overrides(&mut config);
    dynamic_workflows::validate_config(&config)?;
    validate_media_config(&config)?;
    Ok(config)
}

pub fn config_dir() -> PathBuf {
    std::env::var_os(RODER_CONFIG_DIR_ENV)
        .or_else(|| std::env::var_os(RODER_DATA_DIR_ENV))
        .map(PathBuf::from)
        .unwrap_or_else(default_config_dir)
}

pub fn config_file_path() -> PathBuf {
    config_path()
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

pub fn save_search_index_enabled(enabled: bool) -> anyhow::Result<()> {
    save_search_index_enabled_to_path(config_path(), enabled)
}

pub fn save_tools_shell(shell: &str) -> anyhow::Result<()> {
    save_tools_shell_to_path(config_path(), shell)
}

pub fn save_file_backed_dynamic_context(enabled: bool) -> anyhow::Result<()> {
    save_file_backed_dynamic_context_to_path(config_path(), enabled)
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

pub fn delete_provider_api_key(provider: &str) -> anyhow::Result<()> {
    delete_provider_api_key_to_path(config_path(), provider)
}

/**
 * API key persisted for a provider in user config: the stored `api_key`,
 * falling back to the `api_key_env` indirection. Inference engines resolve
 * this per call (after their construction-time key and canonical env var) so
 * keys persisted by `providers/configure` take effect without a process
 * restart.
 */
pub fn provider_api_key(provider: &str) -> Option<String> {
    let config = load_config().ok()?;
    let entry = config.providers.get(provider)?;
    let stored = entry
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    stored.or_else(|| {
        let value = std::env::var(entry.api_key_env.as_deref()?).ok()?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

pub fn save_skills_config(skills: &roder_skills::SkillsConfig) -> anyhow::Result<()> {
    save_skills_config_to_path(config_path(), skills)
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

pub fn save_search_index_enabled_to_path(
    path: impl AsRef<Path>,
    enabled: bool,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config
        .search_index
        .get_or_insert_with(Default::default)
        .enabled = enabled;
    save_config_file_to_path(path, &config)
}

pub fn save_tools_shell_to_path(path: impl AsRef<Path>, shell: &str) -> anyhow::Result<()> {
    let shell = shell.trim();
    if shell.is_empty() {
        anyhow::bail!("tools.shell cannot be empty");
    }
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.tools.get_or_insert_with(Default::default).shell = Some(shell.to_string());
    save_config_file_to_path(path, &config)
}

pub fn save_file_backed_dynamic_context_to_path(
    path: impl AsRef<Path>,
    enabled: bool,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config
        .context
        .get_or_insert_with(Default::default)
        .file_backed_dynamic_context = enabled;
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

pub fn delete_provider_api_key_to_path(
    path: impl AsRef<Path>,
    provider: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    if let Some(p) = config.providers.get_mut(provider) {
        p.api_key = None;
    }
    save_config_file_to_path(path, &config)
}

pub fn save_skills_config_to_path(
    path: impl AsRef<Path>,
    skills: &roder_skills::SkillsConfig,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.skills = Some(skills.clone());
    save_config_file_to_path(path, &config)
}

pub fn build_skills_registry(
    workspace: impl AsRef<Path>,
    skills: Option<&roder_skills::SkillsConfig>,
) -> roder_skills::SkillRegistry {
    let workspace = workspace.as_ref();
    let mut options = roder_skills::SkillRegistryOptions::new(workspace);
    options.config_rules = skills
        .map(|skills| skills.config.clone())
        .unwrap_or_default();
    for (root, canonical_prefix) in local_skill_roots(workspace) {
        options
            .roots
            .push(roder_skills::SkillRoot::workspace(root, canonical_prefix));
    }
    if let Some(home) = dirs::home_dir() {
        options.roots.push(roder_skills::SkillRoot::user(
            home.join(".codex/skills"),
            "user://.codex/skills",
        ));
    }
    if let Ok(store) = marketplaces::load_marketplace_store() {
        for plugin in store.installed_plugins.iter().filter(|plugin| {
            plugin.state == roder_api::marketplace::MarketplaceInstallState::Installed
        }) {
            options.roots.push(roder_skills::SkillRoot::plugin(
                plugin.variant_key.clone(),
                PathBuf::from(&plugin.install_path).join("skills"),
                format!("plugin://{}/skills", plugin.variant_key),
            ));
        }
    }
    let package_paths = packages::PackagePaths::standard(Some(workspace));
    for (package_id, root, canonical_prefix) in packages::package_skill_roots(&package_paths) {
        options.roots.push(roder_skills::SkillRoot::plugin(
            format!("pkg-{package_id}"),
            root,
            canonical_prefix,
        ));
    }
    roder_skills::SkillRegistry::load(options)
}

fn local_skill_roots(workspace: &Path) -> Vec<(PathBuf, String)> {
    let workspace = absolute_path(workspace);
    let project_root = project_root_for_skill_scan(&workspace);
    let mut bases = Vec::new();
    let mut current = workspace.as_path();
    loop {
        bases.push(current.to_path_buf());
        if current == project_root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    bases.reverse();

    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for base in bases {
        for relative in [
            ".agents/skills",
            ".claude/skills",
            ".cursor/skills",
            ".cursor/rules",
        ] {
            let root = base.join(relative);
            if !root.exists() && base != workspace {
                continue;
            }
            let key = root.clone();
            if !seen.insert(key) {
                continue;
            }
            let canonical_prefix =
                format!("workspace://{}", relative_path_string(&project_root, &root));
            roots.push((root, canonical_prefix));
        }
    }
    roots
}

fn project_root_for_skill_scan(workspace: &Path) -> PathBuf {
    let mut cargo_root = None;
    for ancestor in workspace.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
        if ancestor.join("Cargo.toml").exists() {
            cargo_root = Some(ancestor.to_path_buf());
        }
    }
    cargo_root.unwrap_or_else(|| workspace.to_path_buf())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn relative_path_string(from: &Path, to: &Path) -> String {
    if let Ok(relative) = to.strip_prefix(from) {
        return path_to_slash_string(relative);
    }

    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return path_to_slash_string(to);
    }

    let mut relative = PathBuf::new();
    for _ in common..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[common..] {
        relative.push(component.as_os_str());
    }
    path_to_slash_string(&relative)
}

fn path_to_slash_string(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
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
    config_dir().join("config.toml")
}

fn default_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".roder")
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
    if let Some(profile) = env("RODER_RUNTIME_PROFILE")
        && !profile.trim().is_empty()
    {
        config.runtime_profile = Some(profile);
    }
    if let Some(limit) = env("RODER_AUTO_COMPACT_TOKEN_LIMIT")
        && let Ok(limit) = limit.trim().parse::<u32>()
    {
        config.auto_compact_token_limit = Some(limit);
    }
    if let Some(store) = env("RODER_SESSION_STORE")
        && !store.trim().is_empty()
    {
        config.sessions.get_or_insert_with(Default::default).store = store;
    }
    if let Some(url) = env("RODER_POSTGRES_SESSION_URL")
        && !url.trim().is_empty()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .postgres
            .get_or_insert_with(Default::default)
            .database_url = Some(url);
    }
    if let Some(tenant) = env("RODER_POSTGRES_SESSION_TENANT")
        && !tenant.trim().is_empty()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .postgres
            .get_or_insert_with(Default::default)
            .tenant_id = Some(tenant);
    }
    if let Some(max) = env("RODER_POSTGRES_SESSION_MAX_CONNECTIONS")
        && let Ok(max) = max.trim().parse::<u32>()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .postgres
            .get_or_insert_with(Default::default)
            .max_connections = Some(max);
    }
    if let Some(url) = env("RODER_MYSQL_SESSION_URL")
        && !url.trim().is_empty()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .mysql
            .get_or_insert_with(Default::default)
            .database_url = Some(url);
    }
    if let Some(tenant) = env("RODER_MYSQL_SESSION_TENANT")
        && !tenant.trim().is_empty()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .mysql
            .get_or_insert_with(Default::default)
            .tenant_id = Some(tenant);
    }
    if let Some(max) = env("RODER_MYSQL_SESSION_MAX_CONNECTIONS")
        && let Ok(max) = max.trim().parse::<u32>()
    {
        config
            .sessions
            .get_or_insert_with(Default::default)
            .mysql
            .get_or_insert_with(Default::default)
            .max_connections = Some(max);
    }
    if let Some(disabled) = env("RODER_DISABLE_CONTEXT_ARTIFACTS")
        && parse_bool(&disabled).unwrap_or(false)
    {
        config
            .context
            .get_or_insert_with(Default::default)
            .file_backed_dynamic_context = false;
    }
    if let Some(enabled) = env("RODER_FILE_BACKED_DYNAMIC_CONTEXT")
        && let Some(enabled) = parse_bool(&enabled)
    {
        config
            .context
            .get_or_insert_with(Default::default)
            .file_backed_dynamic_context = enabled;
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
    if let Some(mode) = env("RODER_TOOL_SEARCH_MODE")
        && !mode.trim().is_empty()
    {
        config.tool_search.get_or_insert_with(Default::default).mode = Some(mode);
    }
    dynamic_workflows::apply_env_overrides_with(
        config
            .dynamic_workflows
            .get_or_insert_with(Default::default),
        &mut env,
    );
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
    if let Some(scope) = env("RODER_TOOLS_PATH_SCOPE")
        && !scope.trim().is_empty()
    {
        config.tools.get_or_insert_with(Default::default).path_scope = scope;
    }
    if let Some(shell) = env("RODER_TOOLS_SHELL")
        && !shell.trim().is_empty()
    {
        config.tools.get_or_insert_with(Default::default).shell = Some(shell);
    }
    if let Some(disabled) = env("RODER_SEARCH_INDEX_DISABLED")
        && parse_bool(&disabled).unwrap_or(false)
    {
        config
            .search_index
            .get_or_insert_with(Default::default)
            .enabled = false;
    }
    if let Some(max_file_bytes) = env("RODER_SEARCH_INDEX_MAX_FILE_BYTES")
        && let Ok(max_file_bytes) = max_file_bytes.trim().parse::<u64>()
    {
        config
            .search_index
            .get_or_insert_with(Default::default)
            .max_file_bytes = Some(max_file_bytes);
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
    if let Some(enabled) = env("RODER_AUTOMATIONS_ENABLED")
        && let Some(enabled) = parse_bool(&enabled)
    {
        config
            .app_server
            .get_or_insert_with(Default::default)
            .automations
            .enabled = enabled;
    }
    if let Some(server_id) = env("RODER_AUTOMATIONS_SERVER_ID")
        && !server_id.trim().is_empty()
    {
        config
            .app_server
            .get_or_insert_with(Default::default)
            .automations
            .server_id = server_id;
    }
    if let Some(server_role) = env("RODER_AUTOMATIONS_SERVER_ROLE")
        && !server_role.trim().is_empty()
    {
        config
            .app_server
            .get_or_insert_with(Default::default)
            .automations
            .server_role = server_role;
    }
    if let Some(store) = env("RODER_AUTOMATIONS_STORE")
        && !store.trim().is_empty()
    {
        config
            .app_server
            .get_or_insert_with(Default::default)
            .automations
            .store_path = PathBuf::from(store);
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
    if let Some(backend) = env("RODER_MEMORY_BACKEND")
        && !backend.trim().is_empty()
    {
        config.memories.get_or_insert_with(Default::default).backend = Some(backend);
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
    use roder_api::skills::SkillExposure;

    #[test]
    fn serializes_provider_and_model_without_dropping_provider_blocks() {
        let mut config = Config {
            provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("medium".to_string()),
            runtime_profile: None,
            auto_compact_token_limit: None,
            reliability: None,
            speed_policy: None,
            inference_router: None,
            web_search: None,
            tool_search: None,
            dynamic_workflows: None,
            context: None,
            sessions: None,
            subagents: None,
            policy_modes: None,
            commands: None,
            tools: None,
            search_index: None,
            notifications: None,
            tui: None,
            app_server: None,
            remote_runners: None,
            zerolang: None,
            media: None,
            memories: None,
            knowledge: None,
            embedding_providers: HashMap::new(),
            agent_teams: None,
            skills: None,
            providers: HashMap::new(),
            models: HashMap::new(),
            model_profiles: HashMap::new(),
            process_extensions: Vec::new(),
            packages: None,
            analytics: None,
            forks: None,
            agent_nodes: Vec::new(),
        };
        config.providers.insert(
            "openai".to_string(),
            ProviderConfig {
                api_key: Some("key".to_string()),
                ..ProviderConfig::default()
            },
        );

        let encoded = toml::to_string_pretty(&config).unwrap();
        assert!(encoded.contains("provider = \"codex\""));
        assert!(encoded.contains("model = \"gpt-5.5\""));
        assert!(encoded.contains("[providers.openai]"));
        assert!(encoded.contains("api_key = \"key\""));
    }

    #[test]
    fn media_image_generation_config_deserializes_from_toml() {
        let config: Config = toml::from_str(
            r#"
            [media]
            artifacts_dir = "/tmp/artifacts"

            [media.image_generation]
            default_provider = "openai"
            default_model = "gpt-image-2"
            max_outputs = 4
            max_input_images = 16

            [media.image_generation.providers.openai]
            enabled = true
            api_key_env = "OPENAI_API_KEY"
            base_url = "https://api.openai.com/v1"

            [media.image_generation.providers.google]
            enabled = true
            api_key_env = "GEMINI_API_KEY"
            default_model = "gemini-3.1-flash-image"
            "#,
        )
        .unwrap();

        let media = config.media.clone().unwrap();
        let image_generation = media.image_generation.unwrap();
        assert_eq!(image_generation.default_provider.as_deref(), Some("openai"));
        assert_eq!(image_generation.default_model.as_deref(), Some("gpt-image-2"));
        assert_eq!(image_generation.max_outputs, Some(4));
        assert_eq!(image_generation.max_input_images, Some(16));
        let google = image_generation.providers.get("google").unwrap();
        assert_eq!(google.enabled, Some(true));
        assert_eq!(google.api_key_env.as_deref(), Some("GEMINI_API_KEY"));
        assert_eq!(
            google.default_model.as_deref(),
            Some("gemini-3.1-flash-image")
        );
        validate_media_config(&config).unwrap();
    }

    #[test]
    fn media_image_generation_config_validation_rejects_bad_limits_and_models() {
        let config: Config = toml::from_str(
            r#"
            [media.image_generation]
            max_outputs = 0
            "#,
        )
        .unwrap();
        let error = validate_media_config(&config).unwrap_err();
        assert!(error.to_string().contains("max_outputs"));

        let config: Config = toml::from_str(
            r#"
            [media.image_generation]
            default_provider = "google"
            default_model = "gpt-image-2"
            "#,
        )
        .unwrap();
        let error = validate_media_config(&config).unwrap_err();
        assert!(error.to_string().contains("not a known google image model"));

        let config: Config = toml::from_str(
            r#"
            [media.image_generation.providers.openai]
            default_model = "dall-e-1"
            "#,
        )
        .unwrap();
        let error = validate_media_config(&config).unwrap_err();
        assert!(error.to_string().contains("known models: gpt-image-2"));

        // Unknown custom providers may declare models Roder does not know.
        let config: Config = toml::from_str(
            r#"
            [media.image_generation.providers.custom]
            default_model = "my-image-model"
            "#,
        )
        .unwrap();
        validate_media_config(&config).unwrap();
    }

    #[test]
    fn knowledge_config_deserializes_with_defaults() {
        let config: Config = toml::from_str(
            r#"
            [knowledge]
            "#,
        )
        .unwrap();
        let knowledge = config.knowledge.unwrap();
        assert!(knowledge.enabled);
        assert!(knowledge.recall);
        assert_eq!(knowledge.recall_limit, None);
        assert_eq!(knowledge.backend, None);
        assert_eq!(knowledge.store_path, None);

        let config: Config = toml::from_str(
            r#"
            [knowledge]
            enabled = false
            recall = false
            recall_limit = 7
            backend = "markdown"
            store_path = "/tmp/knowledge"
            "#,
        )
        .unwrap();
        let knowledge = config.knowledge.unwrap();
        assert!(!knowledge.enabled);
        assert!(!knowledge.recall);
        assert_eq!(knowledge.recall_limit, Some(7));
        assert_eq!(knowledge.backend.as_deref(), Some("markdown"));
        assert_eq!(knowledge.store_path, Some(PathBuf::from("/tmp/knowledge")));
    }

    #[test]
    fn forks_config_resolves_default_provider_deterministically() {
        let config: Config = toml::from_str(
            r#"
            [forks]
            default_provider = "rift"
            base_dir = "/tmp/forks"
            "#,
        )
        .unwrap();
        assert_eq!(default_fork_provider(&config), "rift");
        assert_eq!(
            config.forks.as_ref().unwrap().base_dir.as_deref(),
            Some("/tmp/forks")
        );

        let empty: Config = toml::from_str("").unwrap();
        assert_eq!(default_fork_provider(&empty), "git-worktree");
    }

    #[test]
    fn deserializes_process_extensions_entries() {
        let config: Config = toml::from_str(
            r#"
            [[process_extensions]]
            id = "python-chat-completions"
            enabled = true
            manifest = "examples/non-rust-extensions/python-chat-completions/roder-extension.toml"
            command = "python3"
            args = ["-m", "roder_python_chat_provider"]
            cwd = "examples/non-rust-extensions/python-chat-completions"
            env = { PYTHONUNBUFFERED = "1" }
            event_filter = { kinds = ["turn.", "inference."] }

            [[process_extensions]]
            id = "disabled-extension"
            enabled = false
            manifest = "missing.toml"
            command = "false"
            "#,
        )
        .unwrap();

        assert_eq!(config.process_extensions.len(), 2);
        let python = &config.process_extensions[0];
        assert_eq!(python.id, "python-chat-completions");
        assert!(python.enabled);
        assert_eq!(python.command, "python3");
        assert_eq!(
            python.env.get("PYTHONUNBUFFERED").map(String::as_str),
            Some("1")
        );
        assert!(python.event_filter.matches("turn.started"));
        assert!(!config.process_extensions[1].enabled);

        let empty: Config = toml::from_str("provider = \"mock\"").unwrap();
        assert!(empty.process_extensions.is_empty());
    }

    #[test]
    fn deserializes_openrouter_provider_attribution_config() {
        let config: Config = toml::from_str(
            r#"
            provider = "openrouter"
            model = "x-ai/grok-build-0.1"

            [providers.openrouter]
            api_key_env = "OPENROUTER_API_KEY"
            base_url = "https://openrouter.ai/api/v1"
            http_referer = "https://example.com"
            app_title = "Roder"
            "#,
        )
        .unwrap();

        let provider = config.providers.get("openrouter").unwrap();
        assert_eq!(config.provider.as_deref(), Some("openrouter"));
        assert_eq!(config.model.as_deref(), Some("x-ai/grok-build-0.1"));
        assert_eq!(provider.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(
            provider.http_referer.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(provider.app_title.as_deref(), Some("Roder"));
    }

    #[test]
    fn deserializes_claude_code_provider_cli_config() {
        let config: Config = toml::from_str(
            r#"
            provider = "claude-code"
            model = "sonnet"

            [providers.claude-code]
            cli_path = "/usr/local/bin/claude"
            permission_mode = "default"
            setting_sources = ["user", "project"]
            "#,
        )
        .unwrap();

        let provider = config.providers.get("claude-code").unwrap();
        assert_eq!(config.provider.as_deref(), Some("claude-code"));
        assert_eq!(config.model.as_deref(), Some("sonnet"));
        assert_eq!(provider.cli_path.as_deref(), Some("/usr/local/bin/claude"));
        assert_eq!(provider.permission_mode.as_deref(), Some("default"));
        assert_eq!(
            provider.setting_sources.as_deref(),
            Some(&["user".to_string(), "project".to_string()][..])
        );
    }

    #[test]
    fn sessions_env_overrides_select_postgres() {
        let mut config = Config::default();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_SESSION_STORE" => Some("postgres".to_string()),
            "RODER_POSTGRES_SESSION_URL" => Some("postgres://user:secret@localhost/db".to_string()),
            "RODER_POSTGRES_SESSION_TENANT" => Some("tenant-a".to_string()),
            "RODER_POSTGRES_SESSION_MAX_CONNECTIONS" => Some("7".to_string()),
            _ => None,
        });

        let sessions = config.sessions.unwrap();
        assert_eq!(sessions.store, "postgres");
        let postgres = sessions.postgres.unwrap();
        assert_eq!(
            postgres.database_url.as_deref(),
            Some("postgres://user:secret@localhost/db")
        );
        assert_eq!(postgres.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(postgres.max_connections, Some(7));
    }

    #[test]
    fn skills_registry_loads_local_agent_claude_and_cursor_roots_from_project_root() {
        let project = temp_config_dir("local-skills-roots");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let workspace = project.join("nested/workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        write_skill(
            &project.join(".agents/skills/agent-local"),
            "agent-local",
            "Agent local skill",
        );
        write_skill(
            &project.join(".claude/skills/claude-local"),
            "claude-local",
            "Claude local skill",
        );
        write_skill(
            &project.join(".cursor/skills/cursor-local"),
            "cursor-local",
            "Cursor local skill",
        );
        std::fs::create_dir_all(project.join(".cursor/rules")).unwrap();
        std::fs::write(
            project.join(".cursor/rules/Review Rule.mdc"),
            "---\ndescription: Cursor rule skill\nalwaysApply: true\n---\n# Review rule\nApply this rule.\n",
        )
        .unwrap();

        let registry = build_skills_registry(&workspace, None);
        let skill_names = registry
            .skills()
            .iter()
            .map(|skill| skill.descriptor.name.as_str())
            .collect::<Vec<_>>();

        assert!(skill_names.contains(&"agent-local"));
        assert!(skill_names.contains(&"claude-local"));
        assert!(skill_names.contains(&"cursor-local"));
        assert!(skill_names.contains(&"review-rule"));

        let cursor_rule = registry
            .skills()
            .iter()
            .find(|skill| skill.descriptor.name == "review-rule")
            .expect("cursor rule skill");
        assert_eq!(cursor_rule.descriptor.exposure, SkillExposure::Global);
        assert!(
            cursor_rule
                .descriptor
                .canonical_path
                .ends_with(".cursor/rules/Review Rule.mdc")
        );
    }

    #[test]
    fn memories_honcho_backend_deserializes_and_env_override_applies() {
        let mut config: Config = toml::from_str(
            r#"
            [memories]
            backend = "sqlite"

            [memories.honcho]
            base_url = "https://honcho.internal"
            workspace_id = "agents"
            peer_id = "memory-writer"
            session_id = "pinned-session"
            api_key_env = "MY_HONCHO_KEY"
            "#,
        )
        .unwrap();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_MEMORY_BACKEND" => Some("honcho".to_string()),
            _ => None,
        });
        let memories = config.memories.unwrap();
        assert_eq!(memories.backend.as_deref(), Some("honcho"));
        let honcho = memories.honcho.unwrap();
        assert_eq!(
            honcho,
            HonchoMemoriesConfig {
                base_url: Some("https://honcho.internal".to_string()),
                workspace_id: Some("agents".to_string()),
                peer_id: Some("memory-writer".to_string()),
                session_id: Some("pinned-session".to_string()),
                api_key_env: Some("MY_HONCHO_KEY".to_string()),
            }
        );
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

            [embedding_providers.google]
            enabled = true
            api_key_env = "GEMINI_API_KEY"
            endpoint = "https://generativelanguage.googleapis.com/v1beta"
            model = "gemini-embedding-2"
            dimensions = 3072

            [embedding_providers.zeroentropy]
            enabled = true
            api_key_env = "ZEROENTROPY_API_KEY"
            endpoint = "https://api.zeroentropy.dev/v1"
            model = "zembed-1"
            dimensions = 2560
            encoding_format = "base64"
            latency = "fast"
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
        let google = &config.embedding_providers["google"];
        assert_eq!(google.api_key_env.as_deref(), Some("GEMINI_API_KEY"));
        assert_eq!(
            google.endpoint.as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(google.model.as_deref(), Some("gemini-embedding-2"));
        assert_eq!(google.dimensions, Some(3072));
        let zeroentropy = &config.embedding_providers["zeroentropy"];
        assert_eq!(
            zeroentropy.api_key_env.as_deref(),
            Some("ZEROENTROPY_API_KEY")
        );
        assert_eq!(
            zeroentropy.endpoint.as_deref(),
            Some("https://api.zeroentropy.dev/v1")
        );
        assert_eq!(zeroentropy.model.as_deref(), Some("zembed-1"));
        assert_eq!(zeroentropy.dimensions, Some(2560));
        assert_eq!(zeroentropy.encoding_format.as_deref(), Some("base64"));
        assert_eq!(zeroentropy.latency.as_deref(), Some("fast"));
    }

    #[test]
    fn context_config_deserializes_saves_and_env_overrides_apply() {
        let mut config: Config = toml::from_str(
            r#"
            [context]
            file_backed_dynamic_context = false
            "#,
        )
        .unwrap();
        assert!(!config.context.as_ref().unwrap().file_backed_dynamic_context);

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_FILE_BACKED_DYNAMIC_CONTEXT" => Some("true".to_string()),
            _ => None,
        });
        assert!(config.context.as_ref().unwrap().file_backed_dynamic_context);

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_DISABLE_CONTEXT_ARTIFACTS" => Some("1".to_string()),
            _ => None,
        });
        assert!(!config.context.as_ref().unwrap().file_backed_dynamic_context);

        let path =
            std::env::temp_dir().join(format!("roder-config-context-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        save_file_backed_dynamic_context_to_path(&path, false).unwrap();
        let saved = load_config_file_from_path(&path).unwrap();
        assert!(!saved.context.unwrap().file_backed_dynamic_context);
        let _ = fs::remove_file(path);
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
    fn runtime_profile_env_override_applies_without_mutating_process_env() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_RUNTIME_PROFILE" => Some("eval".to_string()),
            _ => None,
        });

        assert_eq!(config.runtime_profile.as_deref(), Some("eval"));
    }

    #[test]
    fn speed_policy_config_parses_tunable_thresholds() {
        let config: Config = toml::from_str(
            r#"
            [speed_policy]
            enabled = true
            orientation_reasoning = "high"
            execution_reasoning = "low"
            verification_reasoning = "high"
            recovery_reasoning = "medium"
            eval_deadline_seconds = 600
            "#,
        )
        .unwrap();

        let speed = config.speed_policy.unwrap();
        assert!(speed.enabled);
        assert_eq!(speed.orientation_reasoning.as_deref(), Some("high"));
        assert_eq!(speed.execution_reasoning.as_deref(), Some("low"));
        assert_eq!(speed.verification_reasoning.as_deref(), Some("high"));
        assert_eq!(speed.recovery_reasoning.as_deref(), Some("medium"));
        assert_eq!(speed.eval_deadline_seconds, Some(600));
    }

    #[test]
    fn reliability_config_parses_failure_limits_and_retry_policy() {
        let config: Config = toml::from_str(
            r#"
            [reliability]
            max_consecutive_tool_failures = 4
            max_tool_failures_per_turn = 8
            max_model_calls_per_turn = 20
            provider_retry_max_attempts = 5
            provider_retry_initial_backoff_ms = 250
            provider_retry_backoff_factor = 3
            provider_retry_status_codes = [429, 503]
            retry_empty_provider_body = false
            "#,
        )
        .unwrap();

        let reliability = config.reliability.unwrap();
        assert_eq!(reliability.max_consecutive_tool_failures, Some(4));
        assert_eq!(reliability.max_tool_failures_per_turn, Some(8));
        assert_eq!(reliability.max_model_calls_per_turn, Some(20));
        assert_eq!(reliability.provider_retry_max_attempts, Some(5));
        assert_eq!(reliability.provider_retry_initial_backoff_ms, Some(250));
        assert_eq!(reliability.provider_retry_backoff_factor, Some(3));
        assert_eq!(reliability.provider_retry_status_codes, vec![429, 503]);
        assert_eq!(reliability.retry_empty_provider_body, Some(false));
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
    fn deserializes_tools_shell_and_env_override() {
        let config: Config = toml::from_str(
            r#"
            [tools]
            path_scope = "workspace"
            shell = "zsh"
            allowlist = ["zerolang_check", "zerolang_edit"]
            "#,
        )
        .unwrap();

        let tools = config.tools.unwrap();
        assert_eq!(tools.path_scope, "workspace");
        assert_eq!(tools.shell.as_deref(), Some("zsh"));
        assert_eq!(tools.allowlist, ["zerolang_check", "zerolang_edit"]);

        let mut config = Config::default();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_TOOLS_SHELL" => Some("/bin/bash".to_string()),
            _ => None,
        });

        assert_eq!(config.tools.unwrap().shell.as_deref(), Some("/bin/bash"));
    }

    #[test]
    fn save_tools_shell_persists_tools_section() {
        let path = temp_config_path("tools-shell");
        save_tools_shell_to_path(&path, "bash").unwrap();

        let saved = load_config_file_from_path(&path).unwrap();
        assert_eq!(saved.tools.unwrap().shell.as_deref(), Some("bash"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn deserializes_search_index_config_and_env_overrides() {
        let config: Config = toml::from_str(
            r#"
            [search_index]
            enabled = true
            max_file_bytes = 2097152
            ignored_globs = ["vendor/**", "*.min.js"]
            rebuild_concurrency = 8
            max_index_bytes = 10485760
            "#,
        )
        .unwrap();

        let search_index = config.search_index.unwrap();
        assert!(search_index.enabled);
        assert_eq!(search_index.max_file_bytes, Some(2_097_152));
        assert_eq!(
            search_index.ignored_globs,
            vec!["vendor/**".to_string(), "*.min.js".to_string()]
        );
        assert_eq!(search_index.rebuild_concurrency, Some(8));
        assert_eq!(search_index.max_index_bytes, Some(10_485_760));

        let mut config = Config::default();
        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_SEARCH_INDEX_DISABLED" => Some("true".to_string()),
            "RODER_SEARCH_INDEX_MAX_FILE_BYTES" => Some("4096".to_string()),
            _ => None,
        });
        let search_index = config.search_index.unwrap();
        assert!(!search_index.enabled);
        assert_eq!(search_index.max_file_bytes, Some(4096));
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
    fn automations_config_defaults_keep_scheduler_disabled() {
        let config = AppServerAutomationsConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.server_id, "desktop-main");
        assert_eq!(config.server_role, "desktop");
        assert_eq!(
            config.store_path,
            PathBuf::from("~/.roder/automations.sqlite3")
        );
        assert_eq!(config.tick_seconds, 30);
        assert_eq!(config.lease_seconds, 900);
        assert_eq!(config.max_due_per_tick, 10);
        assert!(config.run_missed_on_startup);
        assert!(config.read_api_when_disabled);
        assert!(config.allowed_project_roots.is_empty());
    }

    #[test]
    fn automations_env_overrides_enable_selected_app_server() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_AUTOMATIONS_ENABLED" => Some("1".to_string()),
            "RODER_AUTOMATIONS_SERVER_ID" => Some("desktop-main".to_string()),
            "RODER_AUTOMATIONS_SERVER_ROLE" => Some("desktop".to_string()),
            "RODER_AUTOMATIONS_STORE" => Some("/tmp/automations.sqlite3".to_string()),
            _ => None,
        });

        let automations = &config.app_server.unwrap().automations;
        assert!(automations.enabled);
        assert_eq!(automations.server_id, "desktop-main");
        assert_eq!(automations.server_role, "desktop");
        assert_eq!(
            automations.store_path,
            PathBuf::from("/tmp/automations.sqlite3")
        );
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
    fn tool_search_mode_env_override_applies_without_mutating_process_env() {
        let mut config = Config::default();

        apply_env_overrides_with(&mut config, |key| match key {
            "RODER_TOOL_SEARCH_MODE" => Some("provider_native".to_string()),
            _ => None,
        });

        assert_eq!(
            config.tool_search.unwrap().mode.as_deref(),
            Some("provider_native")
        );
    }

    #[test]
    fn deserializes_tool_search_config() {
        let config: Config = toml::from_str(
            r#"
            [tool_search]
            mode = "provider_native"
            max_catalog_items = 200
            include_mcp = true
            include_skills = false
            fallback_to_explicit_tools = true
            provider_variant = "bm25"

            [providers.openai.tool_search]
            mode = "provider_native"

            [models."gpt-5.4".tool_search]
            mode = "auto"
            "#,
        )
        .unwrap();

        let tool_search = config.tool_search.unwrap();
        assert_eq!(tool_search.mode.as_deref(), Some("provider_native"));
        assert_eq!(tool_search.max_catalog_items, Some(200));
        assert_eq!(tool_search.include_mcp, Some(true));
        assert_eq!(tool_search.include_skills, Some(false));
        assert_eq!(tool_search.provider_variant.as_deref(), Some("bm25"));
        assert_eq!(
            config
                .providers
                .get("openai")
                .and_then(|provider| provider.tool_search.as_ref())
                .and_then(|tool_search| tool_search.mode.as_deref()),
            Some("provider_native")
        );
        assert_eq!(
            config
                .models
                .get("gpt-5.4")
                .and_then(|model| model.tool_search.as_ref())
                .and_then(|tool_search| tool_search.mode.as_deref()),
            Some("auto")
        );
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
    fn deserializes_zerolang_config() {
        let config: Config = toml::from_str(
            r#"
            [zerolang]
            binary = "/opt/zero/bin/zero"
            timeout_seconds = 45
            artifact_dir = ".zero/roder"
            "#,
        )
        .unwrap();

        let zerolang = config.zerolang.unwrap();
        assert_eq!(
            zerolang.binary.as_deref(),
            Some(Path::new("/opt/zero/bin/zero"))
        );
        assert_eq!(zerolang.timeout_seconds, Some(45));
        assert_eq!(
            zerolang.artifact_dir.as_deref(),
            Some(Path::new(".zero/roder"))
        );
    }

    #[test]
    fn deserializes_model_profile_overrides() {
        let config: Config = toml::from_str(
            r#"
            [model_profiles."gpt-5.5"]
            provider_family = "openai"
            edit_tool = "patch"
            schema_policy = "required_first_flat"
            instruction_overlay = "literal_tool_outputs"
            parallel_tool_calls = true
            auto_compact_token_limit = 180000

            [model_profiles."gpt-5.5".reasoning]
            orientation = "high"
            execution = "low"
            verification = "high"
            recovery = "medium"
            "#,
        )
        .unwrap();

        let profile = config.model_profiles.get("gpt-5.5").unwrap();
        assert_eq!(profile.provider_family.as_deref(), Some("openai"));
        assert_eq!(profile.edit_tool.as_deref(), Some("patch"));
        assert_eq!(
            profile.schema_policy.as_deref(),
            Some("required_first_flat")
        );
        assert_eq!(
            profile.instruction_overlay.as_deref(),
            Some("literal_tool_outputs")
        );
        assert_eq!(profile.parallel_tool_calls, Some(true));
        assert_eq!(profile.auto_compact_token_limit, Some(180000));
        assert_eq!(
            profile
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.execution.as_deref()),
            Some("low")
        );
    }

    #[test]
    fn deserializes_inference_router_config() {
        let config: Config = toml::from_str(
            r#"
            [inference_router]
            enabled = true
            router = "local-adaptive"
            profile = "coding"
            baseline_provider = "codex"
            baseline_model = "gpt-5.5"

            [inference_router.extension]
            objective = "balanced"

            [inference_router.extension.tiers.simple]
            provider = "codex"
            model = "gpt-5.4-mini"
            reasoning = "low"

            [inference_router.extension.tiers.strong]
            provider = "codex"
            model = "gpt-5.5"
            reasoning = "high"

            [inference_router.extension.profiles.coding]
            objective = "cost"
            default_tier = "simple"
            risk_floor_tier = "strong"
            classifier_prompt = "Classify this coding-agent turn."

            [inference_router.extension.profiles.coding.risk_floors]
            security = "strong"
            sandbox = "strong"

            [inference_router.extension.prices."codex/gpt-5.4-mini"]
            input_per_million = 0.25
            output_per_million = 2.0
            cached_input_per_million = 0.025

            [inference_router.extension.classifier_comparison]
            enabled = true
            prompt = "Future classifier comparison prompt"
            label = "classifier-v2"
            "#,
        )
        .unwrap();

        let router = config.inference_router.as_ref().unwrap();
        assert!(router.enabled);
        assert_eq!(router.router.as_deref(), Some("local-adaptive"));
        assert_eq!(router.profile.as_deref(), Some("coding"));
        assert_eq!(router.baseline_provider.as_deref(), Some("codex"));
        assert_eq!(router.baseline_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            router
                .extension
                .get("tiers")
                .and_then(|tiers| tiers.get("simple"))
                .and_then(|tier| tier.get("reasoning"))
                .and_then(serde_json::Value::as_str),
            Some("low")
        );
        assert_eq!(
            router
                .extension
                .get("profiles")
                .and_then(|profiles| profiles.get("coding"))
                .and_then(|profile| profile.get("risk_floors"))
                .and_then(|floors| floors.get("security"))
                .and_then(serde_json::Value::as_str),
            Some("strong")
        );
        assert_eq!(
            router
                .extension
                .get("prices")
                .and_then(|prices| prices.get("codex/gpt-5.4-mini"))
                .and_then(|price| price.get("cached_input_per_million"))
                .and_then(serde_json::Value::as_f64),
            Some(0.025)
        );
        assert_eq!(
            router
                .extension
                .get("classifier_comparison")
                .and_then(|comparison| comparison.get("label"))
                .and_then(serde_json::Value::as_str),
            Some("classifier-v2")
        );
    }

    #[test]
    fn inference_router_config_round_trips_defined_comparison_fields() {
        let mut config = Config::default();
        config.inference_router = Some(InferenceRouterConfig {
            enabled: true,
            router: Some("local-adaptive".to_string()),
            profile: Some("coding".to_string()),
            baseline_provider: Some("codex".to_string()),
            baseline_model: Some("gpt-5.5".to_string()),
            extension: serde_json::json!({
                "objective": "balanced",
                "tiers": {
                    "simple": {
                        "provider": "codex",
                        "model": "gpt-5.4-mini",
                        "reasoning": "low"
                    }
                },
                "classifier_comparison": {
                    "enabled": true,
                    "prompt": "compare later",
                    "label": "future-classifier"
                }
            }),
        });

        let encoded = toml::to_string_pretty(&config).unwrap();
        let decoded: Config = toml::from_str(&encoded).unwrap();
        let router = decoded.inference_router.unwrap();

        assert!(router.enabled);
        assert_eq!(router.router.as_deref(), Some("local-adaptive"));
        assert_eq!(
            router
                .extension
                .get("tiers")
                .and_then(|tiers| tiers.get("simple"))
                .and_then(|tier| tier.get("model"))
                .and_then(serde_json::Value::as_str),
            Some("gpt-5.4-mini")
        );
        assert_eq!(
            router
                .extension
                .get("classifier_comparison")
                .and_then(|comparison| comparison.get("prompt"))
                .and_then(serde_json::Value::as_str),
            Some("compare later")
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
    fn save_default_provider_model_preserves_stored_reasoning_preference() {
        let path = std::env::temp_dir().join(format!(
            "roder-config-clear-reasoning-{}.toml",
            std::process::id()
        ));
        fs::write(
            &path,
            "provider = \"codex\"\nmodel = \"gpt-5.5\"\nreasoning = \"high\"\n",
        )
        .unwrap();

        save_default_provider_model_to_path(&path, "opencode", "big-pickle").unwrap();
        let config = load_config_file_from_path(&path).unwrap();

        assert_eq!(config.provider.as_deref(), Some("opencode"));
        assert_eq!(config.model.as_deref(), Some("big-pickle"));
        assert_eq!(config.reasoning.as_deref(), Some("high"));

        let _ = fs::remove_file(path);
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
    fn save_search_index_enabled_creates_or_updates_search_index_config() {
        let path = std::env::temp_dir().join(format!(
            "roder-config-search-index-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        save_search_index_enabled_to_path(&path, false).unwrap();

        let config = load_config_file_from_path(&path).unwrap();
        assert!(!config.search_index.unwrap().enabled);
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

    #[test]
    fn save_skills_config_creates_canonical_skills_table() {
        let path =
            std::env::temp_dir().join(format!("roder-config-skills-{}.toml", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut skills = roder_skills::SkillsConfig::default();
        skills.upsert_rule(
            roder_api::skills::SkillSelector::Name {
                name: "commit".to_string(),
            },
            |rule| {
                rule.set_enabled(false);
                rule.set_exposure(roder_api::skills::SkillExposure::Global);
            },
        );

        save_skills_config_to_path(&path, &skills).unwrap();

        let saved_text = fs::read_to_string(&path).unwrap();
        assert!(saved_text.contains("[[skills.config]]"));
        assert!(saved_text.contains("name = \"commit\""));
        let config = load_config_file_from_path(&path).unwrap();
        assert_eq!(config.skills.unwrap(), skills);
        let _ = fs::remove_file(&path);
    }

    fn write_skill(dir: &Path, name: &str, description: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nBody for {name}\n"),
        )
        .unwrap();
    }

    fn temp_config_dir(name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("roder-config-{name}-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn temp_config_path(name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("roder-config-{name}-{suffix}.toml"))
    }
}
