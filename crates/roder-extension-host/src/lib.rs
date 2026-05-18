use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use futures::stream;
use roder_api::capabilities::CapabilityRequest;
use roder_api::catalog::{PROVIDER_CODEX, PROVIDER_MOCK, models_for_codex, models_for_provider};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistry, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference::*;
use roder_api::notifications::NotificationKind;
use roder_api::policy_mode::PolicyMode;
use roder_api::remote_runner::RunnerDestination;
use roder_api::tui_status::{PaletteSourceDescriptor, built_in_status_segments};
use roder_ext_anthropic::AnthropicExtension;
use roder_ext_gemini::GeminiExtension;
use roder_ext_jsonl_session::JsonlSessionExtension;
use roder_ext_memory::MemoryExtension;
use roder_ext_openai_embeddings::OpenAiEmbeddingsExtension;
use roder_ext_openai_responses::{OpenAiResponsesEngine, OpenAiResponsesExtension};
use roder_ext_opencode::{OpenCodeConfig, OpenCodeExtension};
use roder_ext_runner_blaxel::BlaxelRunnerExtension;
use roder_ext_runner_cloudflare::CloudflareRunnerExtension;
use roder_ext_runner_daytona::DaytonaRunnerExtension;
use roder_ext_runner_docker::DockerRunnerExtension;
use roder_ext_runner_e2b::E2bRunnerExtension;
use roder_ext_runner_modal::ModalRunnerExtension;
use roder_ext_runner_runloop::RunloopRunnerExtension;
use roder_ext_runner_unix_local::UnixLocalRunnerExtension;
use roder_ext_runner_vercel::VercelRunnerExtension;
use roder_ext_xai::XaiExtension;
use semver::Version;

mod subagents;
mod web_search;
pub mod workflow_import;

pub use subagents::DefaultSubagentsConfig;
pub use web_search::{DefaultWebSearchConfig, DefaultWebSearchProviderConfig};

#[derive(Debug, Clone)]
pub struct DefaultRegistryConfig {
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub xai_base_url: Option<String>,
    pub opencode_api_key: Option<String>,
    pub opencode_base_url: Option<String>,
    pub opencode_project_id: Option<String>,
    pub opencode_go_api_key: Option<String>,
    pub opencode_go_base_url: Option<String>,
    pub opencode_go_project_id: Option<String>,
    pub session_dir: Option<PathBuf>,
    pub workspace: Option<PathBuf>,
    pub web_search: Option<DefaultWebSearchConfig>,
    pub subagents: Option<DefaultSubagentsConfig>,
    pub policy_mode: PolicyMode,
    pub notifications: DefaultNotificationsConfig,
    pub remote_runner_destination: Option<RunnerDestination>,
}

impl Default for DefaultRegistryConfig {
    fn default() -> Self {
        Self {
            openai_api_key: None,
            anthropic_api_key: None,
            gemini_api_key: None,
            xai_api_key: None,
            xai_base_url: None,
            opencode_api_key: None,
            opencode_base_url: None,
            opencode_project_id: None,
            opencode_go_api_key: None,
            opencode_go_base_url: None,
            opencode_go_project_id: None,
            session_dir: None,
            workspace: None,
            web_search: None,
            subagents: None,
            policy_mode: PolicyMode::Default,
            notifications: DefaultNotificationsConfig::default(),
            remote_runner_destination: None,
        }
    }
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

    if let Some(openai_key) = config.openai_api_key {
        builder.install(OpenAiResponsesExtension::new(openai_key))?;
    }
    if let Some(anthropic_key) = config.anthropic_api_key {
        builder.install(AnthropicExtension::new(anthropic_key))?;
    }
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

    builder.install(roder_ext_plan_mode::PlanModeExtension::new(
        config.policy_mode,
    ))?;
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
    builder.install(BuiltinCodingToolsExtension { workspace })?;

    if let Some(subagents) = config.subagents {
        subagents::install_subagents(&mut builder, subagents)?;
    }

    let roder_home = roder_home_dir()?;
    let session_dir = config
        .session_dir
        .unwrap_or_else(|| roder_home.join("sessions"));
    builder.install(JsonlSessionExtension::new(session_dir))?;
    builder.install(MemoryExtension::new(roder_home.join("memory")))?;
    builder.install(OpenAiEmbeddingsExtension::from_env())?;

    builder.build()
}

fn roder_home_dir() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".roder"))
        .context("could not resolve home directory for ~/.roder")
}

struct FakeProviderExtension;

struct EchoToolsExtension;

struct BuiltinCodingToolsExtension {
    workspace: PathBuf,
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
        registry.tool_contributor(roder_tools::builtin_coding_tools_contributor(
            self.workspace.clone(),
        )?);
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
        ("sessions", "Sessions", 90),
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
            ("originator".to_string(), "codex_cli_rs".to_string()),
            (
                "User-Agent".to_string(),
                "codex_cli_rs/0.1.0 roder".to_string(),
            ),
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
        PROVIDER_ANTHROPIC, PROVIDER_GEMINI, PROVIDER_OPENAI, PROVIDER_OPENCODE,
        PROVIDER_OPENCODE_GO, PROVIDER_SUPERGROK, PROVIDER_XAI,
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
            anthropic_api_key: Some("anthropic".to_string()),
            gemini_api_key: Some("gemini".to_string()),
            xai_api_key: Some("xai".to_string()),
            xai_base_url: None,
            opencode_api_key: Some("opencode".to_string()),
            opencode_base_url: None,
            opencode_project_id: None,
            opencode_go_api_key: Some("opencode-go".to_string()),
            opencode_go_base_url: None,
            opencode_go_project_id: None,
            session_dir: None,
            workspace: None,
            web_search: None,
            subagents: None,
            policy_mode: PolicyMode::Default,
            notifications: DefaultNotificationsConfig::default(),
            remote_runner_destination: None,
        })
        .unwrap();
        for provider in [
            PROVIDER_MOCK,
            PROVIDER_OPENAI,
            PROVIDER_CODEX,
            PROVIDER_ANTHROPIC,
            PROVIDER_GEMINI,
            PROVIDER_SUPERGROK,
            PROVIDER_XAI,
            PROVIDER_OPENCODE,
            PROVIDER_OPENCODE_GO,
        ] {
            assert!(
                registry.inference_engine(provider).is_some(),
                "missing {provider}"
            );
        }
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

        for expected in ["mode", "model", "session", "branch", "usage", "mcp"] {
            assert!(
                status_ids.contains(&expected),
                "missing status segment {expected}: {status_ids:?}"
            );
        }
        for expected in ["commands", "sessions", "agents", "models", "modes"] {
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
