use std::path::PathBuf;
use std::sync::Arc;

use futures::stream;
use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::{
    PROVIDER_ANTHROPIC, PROVIDER_CODEX, PROVIDER_CURSOR, PROVIDER_GEMINI, PROVIDER_MOCK,
    PROVIDER_OPENAI, PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO, PROVIDER_OPENROUTER,
    PROVIDER_POOLSIDE, PROVIDER_SUPERGROK, PROVIDER_XAI, PROVIDER_XIAOMI_MIMO,
    PROVIDER_XIAOMI_MIMO_TOKEN_PLAN, models_for_codex, models_for_provider,
};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistry, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference::*;
use roder_api::notifications::NotificationKind;
use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::RunnerDestination;
use roder_api::tui_status::{PaletteSourceDescriptor, built_in_status_segments};
use roder_ext_anthropic::AnthropicExtension;
use roder_ext_chrome::ChromeExtension;
use roder_ext_claude_code::{ClaudeCodeConfig, ClaudeCodeExtension};
use roder_ext_cursor::{CursorConfig, CursorExtension};
use roder_ext_gemini::GeminiExtension;
use roder_ext_git::GitExtension;
use roder_ext_google_speech::{GoogleSpeechConfig, GoogleSpeechExtension};
use roder_ext_jsonl_thread_store::JsonlThreadStoreExtension;
use roder_ext_gbrain::GbrainExtension;
use roder_ext_memory::MemoryExtension;
use roder_ext_openai_embeddings::{OpenAiEmbeddingProvider, OpenAiEmbeddingsExtension};
use roder_ext_openai_responses::{OpenAiResponsesEngine, OpenAiResponsesExtension};
use roder_ext_openai_speech::OpenAiSpeechExtension;
use roder_ext_opencode::{OpenCodeConfig, OpenCodeExtension};
use roder_ext_openrouter::{OpenRouterConfig, OpenRouterExtension};
use roder_ext_poolside::{PoolsideConfig, PoolsideExtension};
use roder_ext_postgres_session::{
    PostgresSessionConfig, PostgresSessionExtension, redact_database_url,
};
use roder_ext_runner_blaxel::BlaxelRunnerExtension;
use roder_ext_runner_cloudflare::CloudflareRunnerExtension;
use roder_ext_runner_daytona::DaytonaRunnerExtension;
use roder_ext_runner_docker::DockerRunnerExtension;
use roder_ext_runner_e2b::E2bRunnerExtension;
use roder_ext_runner_modal::ModalRunnerExtension;
use roder_ext_runner_runloop::RunloopRunnerExtension;
use roder_ext_runner_unix_local::UnixLocalRunnerExtension;
use roder_ext_runner_vercel::VercelRunnerExtension;
use roder_ext_webwright::WebwrightExtension;
use roder_ext_xai::XaiExtension;
use roder_ext_xiaomi_mimo::{XiaomiMimoConfig, XiaomiMimoExtension};
use roder_ext_zerolang::{ZerolangConfig, ZerolangExtension};
use semver::Version;

mod context;
pub mod discovery_catalog;
pub mod marketplace;
mod subagents;
mod web_search;
pub mod workflow_import;

pub use subagents::DefaultSubagentsConfig;
pub use web_search::{DefaultWebSearchConfig, DefaultWebSearchProviderConfig};

#[derive(Debug, Clone)]
pub struct DefaultRegistryConfig {
    pub openai_api_key: Option<String>,
    pub openai_speech_api_key: Option<String>,
    pub google_speech_access_token: Option<String>,
    pub google_speech_api_key: Option<String>,
    pub google_speech_project_id: Option<String>,
    pub google_speech_location: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub claude_code_cli_path: Option<String>,
    pub claude_code_permission_mode: Option<String>,
    pub claude_code_setting_sources: Option<Vec<String>>,
    pub gemini_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub xai_base_url: Option<String>,
    pub opencode_api_key: Option<String>,
    pub opencode_base_url: Option<String>,
    pub opencode_project_id: Option<String>,
    pub opencode_go_api_key: Option<String>,
    pub opencode_go_base_url: Option<String>,
    pub opencode_go_project_id: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub openrouter_base_url: Option<String>,
    pub openrouter_http_referer: Option<String>,
    pub openrouter_app_title: Option<String>,
    pub poolside_api_key: Option<String>,
    pub poolside_base_url: Option<String>,
    pub cursor_api_key: Option<String>,
    pub cursor_access_token: Option<String>,
    pub cursor_agent_service_url: Option<String>,
    pub cursor_backend_base_url: Option<String>,
    pub xiaomi_mimo_api_key: Option<String>,
    pub xiaomi_mimo_base_url: Option<String>,
    pub xiaomi_mimo_token_plan_api_key: Option<String>,
    pub xiaomi_mimo_token_plan_base_url: Option<String>,
    pub custom_inference_providers: Vec<CustomInferenceProviderConfig>,
    pub thread_dir: Option<PathBuf>,
    pub session_store: SessionStoreConfig,
    pub workspace: Option<PathBuf>,
    pub tool_path_scope: roder_tools::ToolPathScope,
    pub command_shell: String,
    pub web_search: Option<DefaultWebSearchConfig>,
    pub subagents: Option<DefaultSubagentsConfig>,
    pub zerolang: Option<ZerolangConfig>,
    pub policy_mode: PolicyMode,
    pub notifications: DefaultNotificationsConfig,
    pub remote_runner_destination: Option<RunnerDestination>,
}

impl Default for DefaultRegistryConfig {
    fn default() -> Self {
        Self {
            openai_api_key: None,
            openai_speech_api_key: None,
            google_speech_access_token: None,
            google_speech_api_key: None,
            google_speech_project_id: None,
            google_speech_location: None,
            anthropic_api_key: None,
            claude_code_cli_path: None,
            claude_code_permission_mode: None,
            claude_code_setting_sources: None,
            gemini_api_key: None,
            xai_api_key: None,
            xai_base_url: None,
            opencode_api_key: None,
            opencode_base_url: None,
            opencode_project_id: None,
            opencode_go_api_key: None,
            opencode_go_base_url: None,
            opencode_go_project_id: None,
            openrouter_api_key: None,
            openrouter_base_url: None,
            openrouter_http_referer: None,
            openrouter_app_title: None,
            poolside_api_key: None,
            poolside_base_url: None,
            cursor_api_key: None,
            cursor_access_token: None,
            cursor_agent_service_url: None,
            cursor_backend_base_url: None,
            xiaomi_mimo_api_key: None,
            xiaomi_mimo_base_url: None,
            xiaomi_mimo_token_plan_api_key: None,
            xiaomi_mimo_token_plan_base_url: None,
            custom_inference_providers: Vec::new(),
            thread_dir: None,
            session_store: SessionStoreConfig::Jsonl,
            workspace: None,
            tool_path_scope: roder_tools::ToolPathScope::default(),
            command_shell: roder_api::command_shell::default_command_shell(),
            web_search: None,
            subagents: None,
            zerolang: None,
            policy_mode: PolicyMode::Default,
            notifications: DefaultNotificationsConfig::default(),
            remote_runner_destination: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SessionStoreConfig {
    #[default]
    Jsonl,
    Postgres(PostgresSessionConfig),
}

#[derive(Debug, Clone)]
pub struct DefaultNotificationsConfig {
    pub enabled: bool,
    pub terminal: bool,
    pub desktop: bool,
    pub enabled_kinds: Vec<NotificationKind>,
}

impl Default for DefaultNotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            terminal: true,
            desktop: true,
            enabled_kinds: vec![
                NotificationKind::NeedsInput,
                NotificationKind::TurnIdle,
                NotificationKind::TaskCompleted,
                NotificationKind::TaskFailed,
            ],
        }
    }
}

pub fn build_default_registry(config: DefaultRegistryConfig) -> anyhow::Result<ExtensionRegistry> {
    let mut builder = ExtensionRegistryBuilder::new();

    builder.install(FakeProviderExtension)?;
    builder.install(CodexOAuthProviderExtension)?;
    builder.install(GitExtension)?;

    builder.install(OpenAiSpeechExtension::new(
        config
            .openai_speech_api_key
            .clone()
            .or_else(|| config.openai_api_key.clone()),
    ))?;
    builder.install(GoogleSpeechExtension::new(GoogleSpeechConfig {
        access_token: config.google_speech_access_token,
        api_key: config.google_speech_api_key,
        project_id: config.google_speech_project_id,
        location: config
            .google_speech_location
            .unwrap_or_else(|| "global".to_string()),
        ..GoogleSpeechConfig::default()
    }))?;

    if let Some(openai_key) = config.openai_api_key {
        builder.install(OpenAiResponsesExtension::new(openai_key))?;
    }
    if let Some(anthropic_key) = config.anthropic_api_key {
        builder.install(AnthropicExtension::new(anthropic_key))?;
    }
    builder.install(ClaudeCodeExtension::new(ClaudeCodeConfig {
        cli_path: config.claude_code_cli_path,
        permission_mode: config.claude_code_permission_mode,
        setting_sources: config.claude_code_setting_sources,
        workspace: config.workspace.clone(),
    }))?;
    if let Some(gemini_key) = config.gemini_api_key {
        builder.install(GeminiExtension::new(gemini_key))?;
    }
    builder.install(XaiExtension::new(config.xai_api_key, config.xai_base_url))?;
    builder.install(OpenCodeExtension::new_with_go(
        OpenCodeConfig {
            api_key: config.opencode_api_key,
            base_url: config.opencode_base_url,
            project_id: config.opencode_project_id,
        },
        OpenCodeConfig {
            api_key: config.opencode_go_api_key,
            base_url: config.opencode_go_base_url,
            project_id: config.opencode_go_project_id,
        },
    ))?;
    builder.install(OpenRouterExtension::new(OpenRouterConfig {
        api_key: config.openrouter_api_key,
        base_url: config.openrouter_base_url,
        http_referer: config.openrouter_http_referer,
        app_title: config.openrouter_app_title,
    }))?;
    builder.install(PoolsideExtension::new(PoolsideConfig {
        api_key: config.poolside_api_key,
        base_url: config.poolside_base_url,
    }))?;
    builder.install(CursorExtension::new(CursorConfig {
        api_key: config.cursor_api_key,
        access_token: config.cursor_access_token,
        agent_service_url: config.cursor_agent_service_url,
        backend_base_url: config.cursor_backend_base_url,
        workspace: config.workspace.clone(),
    }))?;
    builder.install(XiaomiMimoExtension::new(XiaomiMimoConfig {
        api_key: config.xiaomi_mimo_api_key,
        base_url: config.xiaomi_mimo_base_url,
        token_plan_api_key: config.xiaomi_mimo_token_plan_api_key,
        token_plan_base_url: config.xiaomi_mimo_token_plan_base_url,
    }))?;
    for provider in config.custom_inference_providers {
        if known_provider_id(&provider.id) {
            continue;
        }
        builder.inference_engine(Arc::new(OpenAiResponsesEngine::new_custom_provider(
            provider.api_key,
            provider.id.clone(),
            provider.name.unwrap_or(provider.id),
            provider.base_url,
        )));
    }

    builder.install(roder_ext_plan_mode::PlanModeExtension::new(
        config.policy_mode,
    ))?;
    builder.install(roder_ext_task_ledger::TaskLedgerExtension)?;
    builder.install(roder_ext_verification::VerificationExtension)?;
    builder.install(UnixLocalRunnerExtension)?;
    builder.install(DockerRunnerExtension)?;
    builder.install(BlaxelRunnerExtension)?;
    builder.install(CloudflareRunnerExtension)?;
    builder.install(DaytonaRunnerExtension)?;
    builder.install(E2bRunnerExtension)?;
    builder.install(ModalRunnerExtension)?;
    builder.install(RunloopRunnerExtension)?;
    builder.install(VercelRunnerExtension)?;
    builder.install(roder_ext_task_process::ProcessTaskExtension)?;
    builder.install(WebwrightExtension)?;
    builder.install(ChromeExtension::new())?;
    builder.install(ZerolangExtension::new(config.zerolang.unwrap_or_default()))?;
    if config.notifications.enabled && config.notifications.terminal {
        builder.install(roder_ext_notify_terminal::TerminalNotifyExtension::new(
            config.notifications.enabled_kinds.clone(),
        ))?;
    }
    if config.notifications.enabled && config.notifications.desktop {
        builder.install(roder_ext_notify_desktop::DesktopNotifyExtension::new(
            config.notifications.enabled_kinds.clone(),
        ))?;
    }
    builder.install(DefaultTuiExtension)?;

    if let Some(web_search) = config.web_search {
        web_search::install_web_search(&mut builder, web_search)?;
    }

    let workspace = config
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    builder.install(EchoToolsExtension)?;
    builder.install(BuiltinCodingToolsExtension {
        workspace: workspace.clone(),
        path_scope: config.tool_path_scope,
        command_shell: config.command_shell,
    })?;
    context::install_context_planner(&mut builder, &workspace);

    if let Some(subagents) = config.subagents {
        subagents::install_subagents(&mut builder, subagents)?;
    }

    let roder_home = roder_home_dir()?;
    context::install_code_index_context_provider(&mut builder, &workspace, &roder_home)?;
    match config.session_store {
        SessionStoreConfig::Jsonl => {
            let thread_dir = config
                .thread_dir
                .unwrap_or_else(|| roder_home.join("threads"));
            builder.install(JsonlThreadStoreExtension::new(thread_dir))?;
        }
        SessionStoreConfig::Postgres(postgres) => {
            postgres.validate().map_err(|err| {
                anyhow::anyhow!(
                    "invalid PostgreSQL session store config for {}: {err}",
                    redact_database_url(&postgres.database_url)
                )
            })?;
            builder.install(PostgresSessionExtension::new(postgres))?;
        }
    }
    builder.install(MemoryExtension::new(roder_home.join("memory")))?;
    // Bi-temporal gbrain memory: registered AFTER MemoryExtension so the default
    // sqlite-memory store stays primary (`memory_stores.first()`). Uses real
    // OpenAI embeddings when a key is present, deterministic local otherwise.
    let gbrain_embedder = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .map(|key| {
            Arc::new(OpenAiEmbeddingProvider::new(Some(key)))
                as Arc<dyn roder_api::embeddings::EmbeddingProvider>
        });
    builder.install(GbrainExtension::new(
        roder_home.join("gbrain"),
        gbrain_embedder,
    ))?;
    builder.install(OpenAiEmbeddingsExtension::from_env())?;

    builder.build()
}

#[derive(Debug, Clone)]
pub struct CustomInferenceProviderConfig {
    pub id: String,
    pub name: Option<String>,
    pub api_key: Option<String>,
    pub base_url: String,
}

fn known_provider_id(id: &str) -> bool {
    matches!(
        id,
        PROVIDER_MOCK
            | PROVIDER_OPENAI
            | PROVIDER_CODEX
            | PROVIDER_ANTHROPIC
            | PROVIDER_GEMINI
            | PROVIDER_XAI
            | PROVIDER_SUPERGROK
            | PROVIDER_OPENCODE
            | PROVIDER_OPENCODE_GO
            | PROVIDER_OPENROUTER
            | PROVIDER_POOLSIDE
            | PROVIDER_CURSOR
            | PROVIDER_XIAOMI_MIMO
            | PROVIDER_XIAOMI_MIMO_TOKEN_PLAN
    )
}

fn roder_home_dir() -> anyhow::Result<PathBuf> {
    let path = roder_config::config_dir();
    if path.as_os_str().is_empty() {
        anyhow::bail!("configured Roder directory cannot be empty");
    }
    Ok(path)
}

struct FakeProviderExtension;

struct EchoToolsExtension;

struct BuiltinCodingToolsExtension {
    workspace: PathBuf,
    path_scope: roder_tools::ToolPathScope,
    command_shell: String,
}

struct DefaultTuiExtension;

impl RoderExtension for EchoToolsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-builtin-echo-tools".to_string(),
            name: "Built-in Echo Tools".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Offline echo tool provider".to_string()),
            provides: vec![ProvidedService::ToolProvider("builtin-echo".to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(roder_tools::echo_tool_contributor());
        Ok(())
    }
}

impl RoderExtension for BuiltinCodingToolsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-builtin-coding-tools".to_string(),
            name: "Built-in Coding Tools".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Workspace file, search, patch, and command tools".to_string()),
            provides: vec![ProvidedService::ToolProvider(
                "builtin-coding-tools".to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("fs.read.workspace"),
                CapabilityRequest::new("fs.write.workspace"),
                CapabilityRequest::new("process.spawn.shell"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(
            roder_tools::builtin_coding_tools_contributor_with_path_scope_and_shell(
                self.workspace.clone(),
                self.path_scope,
                self.command_shell.clone(),
            )?,
        );
        Ok(())
    }
}

impl RoderExtension for DefaultTuiExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-default-tui".to_string(),
            name: "Default TUI Surfaces".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Built-in status line and command palette sources".to_string()),
            provides: built_in_tui_services(),
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        for segment in built_in_status_segments() {
            registry.status_segment(segment);
        }
        for source in built_in_palette_sources() {
            registry.palette_source(source);
        }
        Ok(())
    }
}

fn built_in_tui_services() -> Vec<ProvidedService> {
    built_in_status_segments()
        .into_iter()
        .map(|segment| ProvidedService::StatusSegment(segment.id))
        .chain(
            built_in_palette_sources()
                .into_iter()
                .map(|source| ProvidedService::PaletteSource(source.id)),
        )
        .collect()
}

fn built_in_palette_sources() -> Vec<PaletteSourceDescriptor> {
    [
        ("commands", "Commands", 100),
        ("threads", "Threads", 90),
        ("agents", "Agents", 80),
        ("models", "Models", 70),
        ("modes", "Modes", 60),
        ("workflow-imports", "Workflow Imports", 55),
        ("media", "Media", 50),
    ]
    .into_iter()
    .map(|(id, label, priority)| PaletteSourceDescriptor {
        id: id.to_string(),
        label: label.to_string(),
        priority,
    })
    .collect()
}

impl RoderExtension for FakeProviderExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-mock-provider".to_string(),
            name: "Mock Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Deterministic local provider for tests and offline development".to_string(),
            ),
            provides: vec![ProvidedService::InferenceEngine(PROVIDER_MOCK.to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        Ok(())
    }
}

struct CodexOAuthProviderExtension;

impl RoderExtension for CodexOAuthProviderExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-codex-oauth-provider".to_string(),
            name: "Codex OAuth Provider".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Codex provider backed by ChatGPT OAuth".to_string()),
            provides: vec![ProvidedService::InferenceEngine(PROVIDER_CODEX.to_string())],
            required_capabilities: vec![],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_engine(Arc::new(CodexOAuthInferenceEngine));
        Ok(())
    }
}

struct CodexOAuthInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for CodexOAuthInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        PROVIDER_CODEX.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: true,
            structured_output: true,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Codex".to_string(),
            description: Some("ChatGPT account provider for Codex models".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("ChatGPT Plus/Pro".to_string()),
            auth_configured: None,
            recommended: true,
            sort_order: 10,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_codex(false))
    }

    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some((access_token, account_id)) = roder_codex_auth::access_token().await? else {
            anyhow::bail!("codex auth is missing; run `roder auth login codex`")
        };
        let mut headers = vec![
            ("originator".to_string(), "roder".to_string()),
            ("User-Agent".to_string(), "roder/0.1.0".to_string()),
        ];
        if let Some(account_id) = account_id {
            headers.push(("ChatGPT-Account-Id".to_string(), account_id));
        }
        OpenAiResponsesEngine::new_with_config(
            access_token,
            PROVIDER_CODEX,
            "https://chatgpt.com/backend-api/codex",
            headers,
        )
        .stream_turn(ctx, request)
        .await
    }
}

struct FakeInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for FakeInferenceEngine {
    fn id(&self) -> roder_api::extension::InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Mock".to_string(),
            description: Some("Local deterministic provider for tests".to_string()),
            auth_type: ProviderAuthType::None,
            auth_label: None,
            auth_configured: Some(true),
            recommended: false,
            sort_order: 1_000,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " from".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: " roder".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use roder_api::catalog::{
        PROVIDER_ANTHROPIC, PROVIDER_CURSOR, PROVIDER_GEMINI, PROVIDER_OPENAI, PROVIDER_OPENCODE,
        PROVIDER_OPENCODE_GO, PROVIDER_POOLSIDE, PROVIDER_SUPERGROK, PROVIDER_XAI,
        PROVIDER_XIAOMI_MIMO, PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
    };
    use roder_api::interactive::{
        HandlerOutcome, HoverCursor, InteractiveEvent, InteractiveRegion, InteractiveRegionHandler,
        RegionKind, RegionRect,
    };

    struct FakeInteractiveExtension {
        calls: Arc<AtomicUsize>,
    }

    impl RoderExtension for FakeInteractiveExtension {
        fn manifest(&self) -> ExtensionManifest {
            ExtensionManifest {
                id: "roder-ext-test-interactive".to_string(),
                name: "Test Interactive Regions".to_string(),
                version: Version::new(0, 1, 0),
                api_version: "0.1.0".to_string(),
                description: None,
                provides: vec![ProvidedService::InteractiveRegionHandler(
                    "test-composer-handler".to_string(),
                )],
                required_capabilities: vec![],
            }
        }

        fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
            registry.interactive_region_handler(Arc::new(FakeInteractiveRegionHandler {
                calls: Arc::clone(&self.calls),
            }));
            Ok(())
        }
    }

    struct FakeInteractiveRegionHandler {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl InteractiveRegionHandler for FakeInteractiveRegionHandler {
        fn id(&self) -> String {
            "test-composer-handler".to_string()
        }

        fn kinds(&self) -> &[&'static str] {
            &["Composer"]
        }

        async fn handle(
            &self,
            _event: InteractiveEvent,
            _region: &InteractiveRegion,
        ) -> anyhow::Result<HandlerOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HandlerOutcome::Consumed)
        }
    }

    #[test]
    fn default_registry_without_keys_has_mock_provider() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        assert!(registry.inference_engine(PROVIDER_MOCK).is_some());
    }

    #[test]
    fn default_registry_installs_speech_transcribers_without_keys() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(registry.speech_transcriber("openai-speech").is_some());
        assert!(registry.speech_transcriber("google-speech").is_some());
    }

    #[test]
    fn default_registry_installs_git_vcs_provider() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(registry.version_control_provider("git").is_some());
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::VersionControlProvider("git".to_string()))
        );
    }

    #[test]
    fn default_registry_installs_webwright_task_and_tools() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::TaskExecutor(
                    "webwright.browser_task".to_string()
                ))
        );
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("webwright".to_string()))
        );
    }

    #[test]
    fn default_registry_installs_zerolang_tools_without_zero_binary() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::ToolProvider("zerolang".to_string()))
        );
        let mut tool_registry = roder_api::tools::ToolRegistry::default();
        for contributor in &registry.tools {
            contributor.contribute(&mut tool_registry).unwrap();
        }
        let names = tool_registry
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"zerolang_edit".to_string()));
        assert!(!names.contains(&"zeolang_edit".to_string()));
    }

    #[test]
    fn default_roder_home_dir_uses_home_roder() {
        let rendered = roder_home_dir()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert!(rendered.ends_with("/.roder"));
        assert!(!rendered.ends_with("/w/.roder"));
    }

    #[test]
    fn default_registry_with_keys_has_gode_provider_ids() {
        let registry = build_default_registry(DefaultRegistryConfig {
            openai_api_key: Some("openai".to_string()),
            openai_speech_api_key: None,
            google_speech_access_token: None,
            google_speech_api_key: None,
            google_speech_project_id: None,
            google_speech_location: None,
            anthropic_api_key: Some("anthropic".to_string()),
            claude_code_cli_path: None,
            claude_code_permission_mode: None,
            claude_code_setting_sources: None,
            gemini_api_key: Some("gemini".to_string()),
            xai_api_key: Some("xai".to_string()),
            xai_base_url: None,
            opencode_api_key: Some("opencode".to_string()),
            opencode_base_url: None,
            opencode_project_id: None,
            opencode_go_api_key: Some("opencode-go".to_string()),
            opencode_go_base_url: None,
            opencode_go_project_id: None,
            openrouter_api_key: Some("openrouter".to_string()),
            openrouter_base_url: None,
            openrouter_http_referer: None,
            openrouter_app_title: None,
            poolside_api_key: Some("poolside".to_string()),
            poolside_base_url: None,
            cursor_api_key: Some("cursor".to_string()),
            cursor_access_token: None,
            cursor_agent_service_url: None,
            cursor_backend_base_url: None,
            xiaomi_mimo_api_key: Some("mimo".to_string()),
            xiaomi_mimo_base_url: None,
            xiaomi_mimo_token_plan_api_key: Some("tp-mimo".to_string()),
            xiaomi_mimo_token_plan_base_url: Some(
                "https://token-plan-cn.xiaomimimo.com/v1".to_string(),
            ),
            custom_inference_providers: Vec::new(),
            thread_dir: None,
            session_store: SessionStoreConfig::Jsonl,
            workspace: None,
            tool_path_scope: roder_tools::ToolPathScope::default(),
            command_shell: "bash".to_string(),
            web_search: None,
            subagents: None,
            zerolang: None,
            policy_mode: PolicyMode::Default,
            notifications: DefaultNotificationsConfig::default(),
            remote_runner_destination: None,
        })
        .unwrap();
        for provider in [
            PROVIDER_MOCK,
            PROVIDER_OPENAI,
            PROVIDER_CODEX,
            roder_api::catalog::PROVIDER_CLAUDE_CODE,
            PROVIDER_ANTHROPIC,
            PROVIDER_GEMINI,
            PROVIDER_SUPERGROK,
            PROVIDER_XAI,
            PROVIDER_OPENCODE,
            PROVIDER_OPENCODE_GO,
            PROVIDER_OPENROUTER,
            PROVIDER_POOLSIDE,
            PROVIDER_CURSOR,
            PROVIDER_XIAOMI_MIMO,
            PROVIDER_XIAOMI_MIMO_TOKEN_PLAN,
        ] {
            assert!(
                registry.inference_engine(provider).is_some(),
                "missing {provider}"
            );
        }
    }

    #[test]
    fn default_registry_installs_custom_openai_compatible_providers() {
        let registry = build_default_registry(DefaultRegistryConfig {
            custom_inference_providers: vec![CustomInferenceProviderConfig {
                id: "local-openai".to_string(),
                name: Some("Local OpenAI".to_string()),
                api_key: None,
                base_url: "http://127.0.0.1:8080".to_string(),
            }],
            ..DefaultRegistryConfig::default()
        })
        .unwrap();

        let provider = registry
            .inference_engine("local-openai")
            .expect("custom provider should be registered");
        let metadata = provider.metadata();
        assert_eq!(metadata.name, "Local OpenAI");
        assert_eq!(metadata.auth_configured, Some(false));
    }

    #[test]
    fn default_registry_installs_xiaomi_speech_synthesis_surfaces_without_keys() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(registry.inference_engine(PROVIDER_XIAOMI_MIMO).is_some());
        assert!(
            registry
                .inference_engine(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN)
                .is_some()
        );
        assert!(registry.speech_synthesizer(PROVIDER_XIAOMI_MIMO).is_some());
        assert!(
            registry
                .speech_synthesizer(PROVIDER_XIAOMI_MIMO_TOKEN_PLAN)
                .is_some()
        );
    }

    #[test]
    fn default_registry_exposes_supergrok_without_xai_api_key() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();

        assert!(registry.inference_engine(PROVIDER_SUPERGROK).is_some());
        assert!(registry.inference_engine(PROVIDER_XAI).is_none());
    }

    #[test]
    fn default_registry_installs_builtin_coding_tools() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let mut tool_registry = roder_api::tools::ToolRegistry::default();
        for contributor in &registry.tools {
            contributor.contribute(&mut tool_registry).unwrap();
        }
        let names = tool_registry
            .specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        for expected in [
            "read_file",
            "list_files",
            "grep",
            "glob",
            "apply_patch",
            "write_file",
            "edit",
            "multi_edit",
            "task_ledger.update",
            "verification_review",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing builtin coding tool {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn tui_integration_default_registry_installs_tui_surfaces() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let status_ids = registry
            .status_segments
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<Vec<_>>();
        let palette_ids = registry
            .palette_sources
            .iter()
            .map(|source| source.id.as_str())
            .collect::<Vec<_>>();

        for expected in ["mode", "model", "thread", "branch", "usage", "mcp"] {
            assert!(
                status_ids.contains(&expected),
                "missing status segment {expected}: {status_ids:?}"
            );
        }
        for expected in ["commands", "threads", "agents", "models", "modes"] {
            assert!(
                palette_ids.contains(&expected),
                "missing palette source {expected}: {palette_ids:?}"
            );
        }
        let services = registry.provided_services();
        assert!(services.iter().any(|service| {
            matches!(service, ProvidedService::StatusSegment(id) if id == "mode")
        }));
        assert!(services.iter().any(|service| {
            matches!(service, ProvidedService::PaletteSource(id) if id == "commands")
        }));
    }

    #[tokio::test]
    async fn mouse_integration_extension_installs_interactive_region_handler() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = ExtensionRegistryBuilder::new();
        builder
            .install(FakeInteractiveExtension {
                calls: Arc::clone(&calls),
            })
            .unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.interactive_region_handlers.len(), 1);
        assert!(
            registry
                .provided_services()
                .contains(&ProvidedService::InteractiveRegionHandler(
                    "test-composer-handler".to_string()
                ))
        );

        let region = InteractiveRegion {
            id: "composer".to_string(),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Composer,
            hover_cursor: HoverCursor::Text,
            keyboard_binding: None,
        };

        let outcome = registry.interactive_region_handlers[0]
            .handle(
                InteractiveEvent::HoverEnter {
                    region: "composer".to_string(),
                },
                &region,
            )
            .await
            .unwrap();

        assert_eq!(outcome, HandlerOutcome::Consumed);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
