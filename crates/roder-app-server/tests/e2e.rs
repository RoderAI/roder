use base64::Engine;
use futures::{SinkExt, StreamExt, stream};
use roder_api::artifacts::ContextArtifactKind;
use roder_api::automations::{
    AutomationConcurrencyPolicy, AutomationProject, AutomationRunState, AutomationSchedule,
    CatchUpPolicy,
};
use roder_api::capabilities::CapabilityDecision;
use roder_api::catalog::{
    PROVIDER_CODEX, PROVIDER_CURSOR, PROVIDER_MOCK, PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO,
    PROVIDER_POOLSIDE, PROVIDER_SUPERGROK, PROVIDER_XAI,
};
use roder_api::code_index::CodeIndexStatus;
use roder_api::discovery::DiscoverySourceKind;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::*;
use roder_api::marketplace::MarketplaceInstallState;
use roder_api::media::{MediaDimensions, MediaGenerationRequest, MediaKind};
use roder_api::memory::MemoryScope;
use roder_api::plan_review::{
    HunkDiffLine, HunkDiffLineKind, HunkRecord, HunkRollbackState, PlanCommentAnchor, PlanReview,
    PlanReviewStatus,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::retrieval::{
    RetrievalConfidence, RetrievalIntent, RetrievalMeasuredOutcome, RetrievalMode,
    RetrievalOutcomeKind, RetrievalPromotionSkipped, RetrievalRecommendation, RetrievalResultUsed,
    RetrievalRouteAccepted, RetrievalRouteIgnored, RetrievalRoutePlan, RetrievalRoutePlanned,
};
use roder_api::skills::{SkillActivationState, SkillExposure, SkillSelector};
use roder_api::subagents::{SubagentDefinition, SubagentPermissionMode};
use roder_api::tasks::TaskState;
use roder_api::thread::{ThreadMetadata, ThreadSnapshot, ThreadStore, ThreadStoreFactory};
use roder_app_server::remote::{
    RemoteServerOptions, RemoteToken, listen_remote_websocket, listen_remote_websocket_controller,
};
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::{
    PendingPlanExit, Runtime, RuntimeConfig, fake_provider::FakeInferenceEngine,
    media_artifacts::MediaArtifactStore,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use roder_ext_memory::MemoryExtension;
use roder_ext_openai_embeddings::OpenAiEmbeddingsExtension;
use roder_ext_subagents::{
    InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry, SubagentsExtension,
};
use roder_extension_host::{
    DefaultRegistryConfig, DefaultWebSearchConfig, DefaultWebSearchProviderConfig,
    build_default_registry,
};
use roder_protocol::{
    AgentsListResult, ArtifactDeleteParams, ArtifactDeleteResult, ArtifactGrepParams,
    ArtifactGrepResult, ArtifactListParams, ArtifactListResult, ArtifactReadParams,
    ArtifactReadResult, ArtifactTailParams, ArtifactTailResult, AutomationsCreateParams,
    AutomationsCreateResult, AutomationsListResult, AutomationsRunNowParams,
    AutomationsRunNowResult, AutomationsRunsParams, AutomationsRunsResult, AutomationsStatusResult,
    CodeIndexProofsListParams, CodeIndexProofsListResult, CodeIndexReadChunkParams,
    CodeIndexReadChunkResult, CodeIndexRebuildParams, CodeIndexRebuildResult,
    CodeIndexSearchParams, CodeIndexSearchResultEnvelope, CodeIndexStatusParams,
    CodeIndexStatusResult, CommandsExpandParams, CommandsExpandResult, CommandsListResult,
    CommandsRunParams, CommandsRunResult, DiscoveryGroupsParams, DiscoveryGroupsResult,
    DiscoveryPromoteParams, DiscoveryPromoteResult, DiscoveryPromotedClearParams,
    DiscoveryPromotedClearResult, DiscoveryPromotedListParams, DiscoveryPromotedListResult,
    DiscoveryReadParams, DiscoveryReadResult, DiscoveryRefreshResult, DiscoverySearchParams,
    DiscoverySearchResult, ExtensionsListResult, HunkListParams, HunkListResult, HunkReadParams,
    HunkReadResult, HunkRollbackParams, HunkRollbackResult, InitializeResult, JsonRpcError,
    JsonRpcRequest, MarketplacesAddParams, MarketplacesAddResult, MarketplacesListResult,
    MarketplacesRefreshParams, MarketplacesRefreshResult, MarketplacesRemoveParams,
    MarketplacesRemoveResult, MarketplacesSearchParams, MarketplacesSearchResult,
    MediaAttachToTurnParams, MediaAttachToTurnResult, MediaDeleteParams, MediaDeleteResult,
    MediaListParams, MediaListResult, MediaReadParams, MediaReadResult, MediaThumbnailParams,
    MediaThumbnailResult, MemoryDeleteParams, MemoryDeleteResult, MemoryListParams,
    MemoryListResult, MemoryProviderListResult, MemoryQueryParams, MemoryQueryResult,
    MemoryReadParams, MemoryReadResult, MemoryRecallPreviewParams, MemoryRecallPreviewResult,
    MemorySaveParams, MemorySaveResult, MemoryUpdateParams, PlanReviewApproveParams,
    PlanReviewCommentParams, PlanReviewCommentResult, PlanReviewReadParams, PlanReviewReadResult,
    PluginDisableParams, PluginDisableResult, PluginInstallAllVariantsParams,
    PluginInstallAllVariantsResult, PluginInstallParams, PluginInstallResult,
    PluginListInstalledResult, PluginPreviewInstallParams, PluginPreviewInstallResult,
    PluginUninstallParams, PluginUninstallResult, ProcessesGetParams, ProcessesGetResult,
    ProcessesListParams, ProcessesListResult, ProcessesStopAllParams, ProcessesStopAllResult,
    ProcessesStopParams, ProcessesStopResult, ProviderAuthResult, ProviderClearParams,
    ProviderClearResult, ProviderConfigureParams, ProviderConfigureResult, ProviderSelectParams,
    ProviderSelectResult, ProvidersListResult, RetrievalMetricsResult, RetrievalPromotedResult,
    RetrievalRecommendationsResult, RetrievalTurnParams, RunnersDeleteResult, RunnersListResult,
    RunnersSelectParams, RunnersSelectResult, RunnersSessionResult, SearchIndexClearParams,
    SearchIndexClearResult, SearchIndexRebuildParams, SearchIndexRebuildResult,
    SearchIndexStatusParams, SearchIndexStatusResult, SearchIndexStatusState,
    SearchIndexWarmupParams, SearchIndexWarmupResult, SettingsGetResult,
    SettingsSetDefaultModeParams, SettingsSetDefaultModeResult,
    SettingsSetFileBackedDynamicContextParams, SettingsSetFileBackedDynamicContextResult,
    SettingsSetSearchIndexParams, SettingsSetSearchIndexResult, SettingsSetShellParams,
    SettingsSetShellResult, SettingsSetWebSearchParams, SettingsSetWebSearchResult,
    SkillsListResult, SkillsSetEnabledParams, SkillsSetExposureParams, SkillsUpdateResult,
    SubagentTraceReadParams, SubagentTraceReadResult, SubagentTracesListParams,
    SubagentTracesListResult, TasksGetParams, TasksGetResult, TasksListResult, TasksSubmitParams,
    TasksSubmitResult, TeamCleanupParams, TeamCleanupResult, TeamListParams, TeamListResult,
    TeamMemberInterruptParams, TeamMemberInterruptResult, TeamMemberMessageParams,
    TeamMemberMessageResult, TeamMemberStartParams, TeamMemberStartResult, TeamReadParams,
    TeamReadResult, TeamStartMemberParams, TeamStartParams, TeamStartResult, ThreadArchiveParams,
    ThreadArchiveResult, ThreadExitPlanParams, ThreadExitPlanResult, ThreadGoalClearParams,
    ThreadGoalClearResult, ThreadGoalGetParams, ThreadGoalGetResult, ThreadGoalSetParams,
    ThreadGoalSetResult, ThreadGoalStatus, ThreadListParams, ThreadListResult, ThreadReadParams,
    ThreadReadResult, ThreadResolveApprovalParams, ThreadResolveApprovalResult,
    ThreadResolveUserInputParams, ThreadResolveUserInputResult, ThreadSetModeParams,
    ThreadSetModeResult, ThreadStartParams, ThreadStartResult, ThreadStateResult, ToolCallParams,
    ToolCallResult, ToolsListResult, TurnInputItem, TurnInterruptParams, TurnStartParams,
    TurnStartResult, TurnSteerParams, TurnSteerResult, WorkflowEnableParams, WorkflowEnableResult,
    WorkflowPreviewParams, WorkflowPreviewResult, WorkflowScanParams, WorkflowScanResult,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

struct TaskCallingEngine {
    hang_child: bool,
    parent_calls: Mutex<usize>,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

static SEARCH_INDEX_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static DISCOVERY_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static SKILLS_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct PendingEngine;

struct UsageReportingEngine;

struct ImageRecordingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct FailingThreadStoreFactory;

struct FailingThreadStore;

#[derive(Clone, Default)]
struct RecordingThreadStoreFactory {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

struct RecordingThreadStore {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

impl ThreadStoreFactory for FailingThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "failing".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(FailingThreadStore)
    }
}

#[async_trait::async_trait]
impl ThreadStore for FailingThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "failing".to_string()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        anyhow::bail!(
            "parse thread metadata /tmp/roder/threads/bad/metadata.json: trailing characters at line 1 column 450"
        );
    }

    async fn load_thread(
        &self,
        _thread_id: &roder_api::events::ThreadId,
    ) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(None)
    }

    async fn append_event(
        &self,
        _thread_id: &roder_api::events::ThreadId,
        _envelope: &roder_api::events::EventEnvelope,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn append_turn_item(
        &self,
        _thread_id: &roder_api::events::ThreadId,
        _turn_id: &roder_api::events::TurnId,
        _item: &roder_api::transcript::TranscriptItem,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl ThreadStoreFactory for RecordingThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "recording".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(RecordingThreadStore {
            snapshots: Arc::clone(&self.snapshots),
        })
    }
}

#[async_trait::async_trait]
impl ThreadStore for RecordingThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "recording".to_string()
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        self.snapshots.lock().await.insert(
            metadata.thread_id.clone(),
            ThreadSnapshot {
                metadata: Some(metadata.clone()),
                events: Vec::new(),
                turns: Vec::new(),
                extension_states: Vec::new(),
            },
        );
        Ok(metadata)
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        if let Some(snapshot) = self.snapshots.lock().await.get_mut(&metadata.thread_id) {
            snapshot.metadata = Some(metadata.clone());
        }
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(self
            .snapshots
            .lock()
            .await
            .values()
            .filter_map(|snapshot| snapshot.metadata.clone())
            .collect())
    }

    async fn load_thread(
        &self,
        thread_id: &roder_api::events::ThreadId,
    ) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(self.snapshots.lock().await.get(thread_id).cloned())
    }

    async fn archive_thread(
        &self,
        thread_id: &roder_api::events::ThreadId,
    ) -> anyhow::Result<bool> {
        Ok(self.snapshots.lock().await.remove(thread_id).is_some())
    }

    async fn append_event(
        &self,
        thread_id: &roder_api::events::ThreadId,
        envelope: &roder_api::events::EventEnvelope,
    ) -> anyhow::Result<()> {
        if let Some(snapshot) = self.snapshots.lock().await.get_mut(thread_id) {
            if let roder_api::events::RoderEvent::TurnCompleted(event) = &envelope.event
                && let Some(turn) = snapshot
                    .turns
                    .iter_mut()
                    .find(|turn| turn.turn_id == event.turn_id)
            {
                turn.completed_at = Some(event.timestamp);
                turn.usage = event.usage.clone();
            }
            snapshot.events.push(envelope.clone());
        }
        Ok(())
    }

    async fn append_turn_item(
        &self,
        thread_id: &roder_api::events::ThreadId,
        turn_id: &roder_api::events::TurnId,
        item: &roder_api::transcript::TranscriptItem,
    ) -> anyhow::Result<()> {
        let mut snapshots = self.snapshots.lock().await;
        if let Some(snapshot) = snapshots.get_mut(thread_id) {
            if let Some(turn) = snapshot
                .turns
                .iter_mut()
                .find(|turn| turn.turn_id == *turn_id)
            {
                turn.items.push(item.clone());
            } else {
                snapshot.turns.push(roder_api::thread::TurnRecord {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    items: vec![item.clone()],
                    created_at: OffsetDateTime::now_utc(),
                    completed_at: None,
                    usage: None,
                });
            }
        }
        Ok(())
    }
}

struct UserInputEngine {
    calls: Mutex<usize>,
}

struct ApprovalRequiredEngine {
    calls: Mutex<usize>,
}

struct WorkspaceToolsEngine {
    calls: Mutex<usize>,
}

#[async_trait::async_trait]
impl InferenceEngine for PendingEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(stream::pending()))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for UsageReportingEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "usage recorded".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Usage(
                TokenUsage::new(100, 10, 110).with_cached_prompt_tokens(92),
            )),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ImageRecordingEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        let mut capabilities = InferenceCapabilities::coding_agent_default();
        capabilities.image_input = true;
        capabilities
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().await.push(request);
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            }),
        )])))
    }
}

impl TaskCallingEngine {
    fn new(hang_child: bool) -> Self {
        Self {
            hang_child,
            parent_calls: Mutex::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ApprovalRequiredEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut calls = self.calls.lock().await;
        *calls += 1;
        if *calls == 1 {
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "approval-shell-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({ "command": "printf hi" }).to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        Ok(Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "approval handled".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for WorkspaceToolsEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut calls = self.calls.lock().await;
        *calls += 1;
        if *calls == 1 {
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "workspace-pwd".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({ "command": "pwd" }).to_string(),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "workspace-read".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({ "path": "marker.txt" }).to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        Ok(Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "done".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for UserInputEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut calls = self.calls.lock().await;
        *calls += 1;
        if *calls == 1 {
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "user-input-1".to_string(),
                    name: "request_user_input".to_string(),
                    arguments: serde_json::json!({
                        "questions": [{
                            "header": "Mode",
                            "id": "mode",
                            "question": "Which mode?",
                            "options": [
                                { "label": "Safe", "description": "Keep restrictions." },
                                { "label": "Fast", "description": "Allow automation." }
                            ]
                        }]
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        Ok(Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "choice noted".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for TaskCallingEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().await.push(request.clone());
        if request.metadata.get("subagent").is_some() {
            if self.hang_child {
                std::future::pending::<()>().await;
            }
            return Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "child result".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }

        let mut parent_calls = self.parent_calls.lock().await;
        *parent_calls += 1;
        if *parent_calls == 1 {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "task-call-1".to_string(),
                    name: "task".to_string(),
                    arguments: serde_json::json!({
                        "description": "Inspect repository",
                        "prompt": "Report the relevant finding.",
                        "subagent_type": "explore"
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])))
        } else {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "parent final".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ])))
        }
    }
}

#[tokio::test]
async fn test_app_server_e2e() {
    let engine = Arc::new(FakeInferenceEngine);
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.tool_contributor(roder_tools::echo_tool_contributor());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let status: InitializeResult = request(&client, "initialize", None).await;
    assert_eq!(status.provider, PROVIDER_MOCK);
    assert_eq!(status.model, "mock");

    let extensions: ExtensionsListResult = request(&client, "extensions/list", None).await;
    assert!(extensions.extensions.is_empty());

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    assert_eq!(providers.providers.len(), 1);
    assert_eq!(providers.providers[0].id, PROVIDER_MOCK);

    let tools: ToolsListResult = request(&client, "tools/list", None).await;
    assert!(tools.tools.iter().any(|tool| tool.name == "echo"));

    let selected: ProviderSelectResult = request(
        &client,
        "providers/select",
        Some(
            serde_json::to_value(ProviderSelectParams {
                provider: PROVIDER_MOCK.to_string(),
                model: Some("alternate-mock-model".to_string()),
                reasoning: Some("none".to_string()),
                thread_id: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(selected.provider, PROVIDER_MOCK);
    assert_eq!(selected.model, "alternate-mock-model");
    assert_eq!(selected.reasoning, "none");

    let status: InitializeResult = request(&client, "initialize", None).await;
    assert_eq!(status.provider, PROVIDER_MOCK);
    assert_eq!(status.model, "alternate-mock-model");

    let invalid_provider = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("providers/select-invalid")),
            method: "providers/select".to_string(),
            params: Some(
                serde_json::to_value(ProviderSelectParams {
                    provider: "missing-provider".to_string(),
                    model: Some("missing-model".to_string()),
                    reasoning: None,
                    thread_id: None,
                })
                .unwrap(),
            ),
        })
        .await;
    assert!(invalid_provider.result.is_none());
    let error = invalid_provider
        .error
        .expect("missing invalid provider error");
    assert_eq!(error.code, -32000);
    assert!(error.message.contains("missing-provider"));

    let thread_start = start_thread(&client).await;
    assert_eq!(thread_start.model_provider, PROVIDER_MOCK);
    assert_eq!(thread_start.model, "alternate-mock-model");
    assert!(!thread_start.thread.id.is_empty());

    let threads: ThreadListResult = request(
        &client,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
    )
    .await;
    assert!(
        threads
            .data
            .iter()
            .any(|thread| thread.id == thread_start.thread.id)
    );

    let started = start_turn(&client, &thread_start.thread.id, "Hello").await;
    assert!(!started.turn_id.is_empty());

    let mut kinds = Vec::new();
    for _ in 0..20 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(&thread_start.thread.id) {
            kinds.push(envelope.kind);
        }
        if kinds.iter().any(|kind| kind == "turn.completed") {
            break;
        }
    }

    assert!(
        kinds.iter().any(|kind| kind == "turn.started"),
        "missing turn.started: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "inference.started"),
        "missing inference.started: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "inference.event_received"),
        "missing inference.event_received: {kinds:?}"
    );
    assert!(
        kinds.iter().any(|kind| kind == "turn.completed"),
        "missing turn.completed: {kinds:?}"
    );
}

#[tokio::test]
async fn thread_goal_methods_share_state_with_goal_tools() {
    let workspace =
        std::env::temp_dir().join(format!("roder-goal-app-server-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(workspace).unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();
    let thread = start_thread(&client).await.thread;

    let set: ThreadGoalSetResult = request(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some("Ship shared goal state".to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: Some(Some(25)),
            })
            .unwrap(),
        ),
    )
    .await;
    let goal = set.goal.expect("created goal");
    assert_eq!(goal.objective, "Ship shared goal state");
    assert_eq!(goal.status, ThreadGoalStatus::Active);
    assert_eq!(goal.token_budget, Some(25));
    let updated =
        wait_for_notification(&mut notifications, "thread/goal/updated", Some(&thread.id)).await;
    assert_eq!(
        updated.params["goal"]["objective"],
        "Ship shared goal state"
    );

    let tool_get: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "get_goal".to_string(),
                arguments: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!tool_get.is_error);
    assert!(tool_get.text.contains("Ship shared goal state"));

    let tool_update: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread.id.clone(),
                tool_name: "update_goal".to_string(),
                arguments: serde_json::json!({ "status": "blocked" }),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!tool_update.is_error);

    let get: ThreadGoalGetResult = request(
        &client,
        "thread/goal/get",
        Some(
            serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        get.goal.expect("goal after tool update").status,
        ThreadGoalStatus::Blocked
    );

    let invalid = request_error(
        &client,
        "thread/goal/set",
        Some(
            serde_json::to_value(ThreadGoalSetParams {
                thread_id: thread.id.clone(),
                objective: Some(" ".to_string()),
                status: None,
                token_budget: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(invalid.message.contains("goal objective cannot be empty"));

    let clear: ThreadGoalClearResult = request(
        &client,
        "thread/goal/clear",
        Some(
            serde_json::to_value(ThreadGoalClearParams {
                thread_id: thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(clear.cleared);
    wait_for_notification(&mut notifications, "thread/goal/cleared", Some(&thread.id)).await;

    let get: ThreadGoalGetResult = request(
        &client,
        "thread/goal/get",
        Some(
            serde_json::to_value(ThreadGoalGetParams {
                thread_id: thread.id,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(get.goal.is_none());
}

#[tokio::test]
async fn model_switch_providers_select_updates_protocol_thread_model_for_next_turn() {
    let engine = Arc::new(TaskCallingEngine {
        hang_child: false,
        parent_calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": "/tmp",
            "ephemeral": false
        })),
    )
    .await;
    let thread_id = started.thread.id.clone();

    let selected: ProviderSelectResult = request(
        &client,
        "providers/select",
        Some(
            serde_json::to_value(ProviderSelectParams {
                provider: PROVIDER_MOCK.to_string(),
                model: Some("alternate-mock-model".to_string()),
                reasoning: Some("none".to_string()),
                thread_id: Some(thread_id.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(selected.model, "alternate-mock-model");
    assert_eq!(
        selected.model_profile.as_deref(),
        Some("alternate-mock-model")
    );
    assert!(
        selected
            .model_switch_summary
            .as_deref()
            .is_some_and(|summary| {
                summary.contains("previous profile mock/mock")
                    && summary.contains("Current profile mock/alternate-mock-model")
            })
    );

    let _: TurnStartResult = request(
        &client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": "hello" }]
        })),
    )
    .await;

    let request = wait_for_recorded_request(&engine).await;
    assert_eq!(request.model.provider, PROVIDER_MOCK);
    assert_eq!(request.model.model, "alternate-mock-model");
}

#[tokio::test]
async fn roadmap_methods_update_documents_threads_and_notifications() {
    let root =
        std::env::temp_dir().join(format!("roder-roadmap-app-server-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("roadmap")).unwrap();
    std::fs::write(
        root.join("roadmap/20-roadmapping-mode.md"),
        roadmap_fixture(),
    )
    .unwrap();

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(root.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server.clone());
    let mut notifications = client.subscribe_notifications();

    let listed: serde_json::Value = request(&client, "roadmap/list", None).await;
    assert_eq!(listed["documents"].as_array().unwrap().len(), 1);

    let read: serde_json::Value = request(
        &client,
        "roadmap/read",
        Some(serde_json::json!({ "path": "roadmap/20-roadmapping-mode.md" })),
    )
    .await;
    assert_eq!(
        read["document"]["title"],
        "Roadmapping Mode Implementation Plan"
    );
    let task_id = read["document"]["tasks"][0]["id"].as_str().unwrap();

    let created: serde_json::Value = request(
        &client,
        "roadmap/create",
        Some(serde_json::json!({
            "slug": "new-plan",
            "title": "New Plan",
            "goal": "Create a new plan."
        })),
    )
    .await;
    assert_eq!(
        created["path"].as_str().unwrap().replace('\\', "/"),
        "roadmap/21-new-plan.md"
    );

    let patched: serde_json::Value = request(
        &client,
        "roadmap/patch",
        Some(serde_json::json!({
            "path": "roadmap/21-new-plan.md",
            "oldText": "Create a new plan.",
            "newText": "Create a patched plan."
        })),
    )
    .await;
    assert_eq!(patched["changed"], true);

    let updated: serde_json::Value = request(
        &client,
        "roadmap/task/update",
        Some(serde_json::json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "taskId": task_id,
            "checked": true,
            "evidence": "app-server test evidence"
        })),
    )
    .await;
    assert_eq!(updated["checked"], true);

    let opened: serde_json::Value = request(
        &client,
        "thread/roadmap/open",
        Some(serde_json::json!({ "path": "roadmap/20-roadmapping-mode.md" })),
    )
    .await;
    assert_eq!(
        opened["document"]["path"],
        root.join("roadmap/20-roadmapping-mode.md")
            .display()
            .to_string()
    );

    let spawned: serde_json::Value = request(
        &client,
        "roadmap/thread/spawn",
        Some(serde_json::json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "taskId": task_id
        })),
    )
    .await;
    assert!(
        spawned["thread"]["thread_id"]
            .as_str()
            .unwrap()
            .trim()
            .len()
            > 0
    );
    let spawned_thread_id = spawned["thread"]["thread_id"].as_str().unwrap();
    let spawned_thread: serde_json::Value = request(
        &client,
        "thread/read",
        Some(serde_json::json!({
            "threadId": spawned_thread_id,
            "includeTurns": true
        })),
    )
    .await;
    assert_eq!(spawned_thread["thread"]["id"], spawned_thread_id);

    let attached: serde_json::Value = request(
        &client,
        "thread/attach",
        Some(serde_json::json!({
            "path": "roadmap/20-roadmapping-mode.md",
            "taskId": task_id,
            "threadId": "thread-existing",
            "title": "Existing worker"
        })),
    )
    .await;
    assert_eq!(attached["thread"]["thread_id"], "thread-existing");

    let threads: serde_json::Value = request(
        &client,
        "roadmap/thread/list",
        Some(serde_json::json!({ "path": "roadmap/20-roadmapping-mode.md" })),
    )
    .await;
    assert!(threads["threads"].as_array().unwrap().len() >= 2);

    let validation: serde_json::Value = request(
        &client,
        "roadmap/validate",
        Some(serde_json::json!({ "path": "roadmap/20-roadmapping-mode.md" })),
    )
    .await;
    assert!(
        validation["results"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let mut methods = Vec::new();
    for _ in 0..8 {
        let notification = tokio::time::timeout(Duration::from_secs(2), notifications.recv())
            .await
            .unwrap()
            .unwrap();
        methods.push(notification.method);
        if methods
            .iter()
            .any(|method| method == "roadmap/threadChanged")
        {
            break;
        }
    }
    assert!(methods.iter().any(|method| method == "roadmap/changed"));
    assert!(methods.iter().any(|method| method == "roadmap/taskChanged"));
    assert!(
        methods
            .iter()
            .any(|method| method == "roadmap/threadChanged")
    );

    let unsupported = request_error(
        &client,
        "acp/roadmap/open",
        Some(serde_json::json!({ "path": "roadmap/20-roadmapping-mode.md" })),
    )
    .await;
    assert_eq!(unsupported.code, -32601);
}

#[tokio::test]
async fn protocol_turn_uses_thread_cwd_for_workspace_tools() {
    let root = std::env::temp_dir().join(format!("roder-thread-cwd-e2e-{}", uuid::Uuid::new_v4()));
    let process_workspace = root.join("process-workspace");
    let thread_workspace = root.join("thread-workspace");
    std::fs::create_dir_all(&process_workspace).unwrap();
    std::fs::create_dir_all(&thread_workspace).unwrap();
    std::fs::write(process_workspace.join("marker.txt"), "process marker").unwrap();
    std::fs::write(thread_workspace.join("marker.txt"), "thread marker").unwrap();

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(WorkspaceToolsEngine {
        calls: Mutex::new(0),
    }));
    builder.tool_contributor(
        roder_tools::builtin_coding_tools_contributor(&process_workspace).unwrap(),
    );
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(process_workspace.display().to_string()),
                policy_mode: PolicyMode::AcceptAll,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let started: ThreadStartResult = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": thread_workspace.display().to_string(),
            "ephemeral": false
        })),
    )
    .await;

    let _: TurnStartResult = request(
        &client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": started.thread.id,
            "input": [{ "type": "text", "text": "where are you?" }]
        })),
    )
    .await;

    let mut shell_output = None;
    let mut read_output = None;
    for _ in 0..30 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::ToolCallCompleted(event) = envelope.event {
            match event.tool_id.as_str() {
                "workspace-pwd" => shell_output = event.output,
                "workspace-read" => read_output = event.output,
                _ => {}
            }
            if shell_output.is_some() && read_output.is_some() {
                break;
            }
        }
    }

    let shell_output = shell_output.expect("missing shell tool output");
    assert!(
        shell_output.contains(&thread_workspace.display().to_string()),
        "shell should run in thread workspace {thread_workspace:?}, got {shell_output:?}"
    );
    assert!(
        !shell_output.contains(&process_workspace.display().to_string()),
        "shell leaked process workspace {process_workspace:?}: {shell_output:?}"
    );
    let read_output = read_output.expect("missing read_file tool output");
    assert!(read_output.contains("thread marker"));
    assert!(!read_output.contains("process marker"));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn turn_start_passes_image_input_to_model_request() {
    let engine = Arc::new(ImageRecordingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let started = start_thread(&client).await;
    let _: TurnStartResult = request(
        &client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": started.thread.id,
            "input": [
                { "type": "text", "text": "what do you see?" },
                { "type": "image", "imageUrl": "data:image/png;base64,YWJj" }
            ]
        })),
    )
    .await;

    let request = wait_for_image_recorded_request(&engine).await;
    let user_message = request
        .transcript
        .iter()
        .rev()
        .find_map(|item| match item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => Some(message),
            _ => None,
        })
        .expect("user message in inference request");
    assert_eq!(user_message.text, "what do you see?");
    assert_eq!(user_message.images.len(), 1);
    assert_eq!(
        user_message.images[0].image_url,
        "data:image/png;base64,YWJj"
    );
}

#[tokio::test]
async fn workflow_import_methods_scan_preview_and_enable_passive_items() {
    let repo = std::env::temp_dir().join(format!(
        "roder-workflow-app-server-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(repo.join(".agents/skills/demo")).unwrap();
    std::fs::write(repo.join("AGENTS.md"), "Use the repo AGENTS instructions.").unwrap();
    std::fs::write(
        repo.join(".agents/skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Demo skill\n---\nBody",
    )
    .unwrap();
    let state_path = repo.join("roder-home").join("workflow-imports.json");
    unsafe {
        std::env::set_var("RODER_WORKFLOW_IMPORTS_PATH", &state_path);
    }

    let registry = build_default_registry(DefaultRegistryConfig {
        workspace: Some(repo.clone()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let scan: WorkflowScanResult = request(
        &client,
        "workflow/scan",
        Some(
            serde_json::to_value(WorkflowScanParams {
                workspace: Some(repo.display().to_string()),
                include_user: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(scan.scan.errors.is_empty());
    let guidance = scan
        .scan
        .items
        .iter()
        .find(|item| item.title == "AGENTS.md")
        .unwrap();
    assert!(!guidance.approval_required);

    let preview: WorkflowPreviewResult = request(
        &client,
        "workflow/preview",
        Some(
            serde_json::to_value(WorkflowPreviewParams {
                workspace: Some(repo.display().to_string()),
                item_id: Some(guidance.id.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(preview.items.len(), 1);

    let enabled: WorkflowEnableResult = request(
        &client,
        "workflow/enable",
        Some(
            serde_json::to_value(WorkflowEnableParams {
                workspace: Some(repo.display().to_string()),
                item_id: guidance.id.clone(),
                approve_side_effects: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(enabled.item.id, guidance.id);
    assert!(state_path.exists());
}

#[tokio::test]
async fn marketplace_methods_add_refresh_search_preview_and_install_plugins() {
    let root = std::env::temp_dir().join(format!(
        "roder-marketplace-app-server-{}",
        uuid::Uuid::new_v4()
    ));
    let store_path = root.join("state").join("marketplaces.json");
    let cache_dir = root.join("cache");
    let cursor_marketplace_root = root.join("cursor-marketplace");
    let claude_marketplace_root = root.join("claude-marketplace");
    std::fs::create_dir_all(cursor_marketplace_root.join(".cursor-plugin")).unwrap();
    std::fs::create_dir_all(claude_marketplace_root.join(".claude-plugin")).unwrap();
    std::fs::write(
        cursor_marketplace_root
            .join(".cursor-plugin")
            .join("marketplace.json"),
        serde_json::json!({
            "plugins": [
                {
                    "id": "repo-tools",
                    "name": "Repo Tools",
                    "description": "Repository helper skills",
                    "repository": "https://github.com/example/repo-tools",
                    "tags": ["git", "repo"],
                    "skills": ["review"],
                    "mcp": true
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        claude_marketplace_root
            .join(".claude-plugin")
            .join("marketplace.json"),
        serde_json::json!({
            "plugins": [
                {
                    "name": "repo-tools-claude",
                    "displayName": "Repo Tools",
                    "description": "Repository helper skills for Claude",
                    "repository": "https://github.com/example/repo-tools",
                    "source": "repo-tools-claude",
                    "skills": ["review"],
                    "tags": ["git", "repo"]
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    unsafe {
        std::env::set_var("RODER_MARKETPLACES_PATH", &store_path);
        std::env::set_var("RODER_MARKETPLACE_CACHE_DIR", &cache_dir);
    }

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let added: MarketplacesAddResult = request(
        &client,
        "marketplaces/add",
        Some(
            serde_json::to_value(MarketplacesAddParams {
                id: "cursor-local".to_string(),
                kind: Some(roder_api::marketplace::MarketplaceKind::Cursor),
                display_name: "Cursor Local".to_string(),
                source: roder_api::marketplace::MarketplaceSource::LocalPath {
                    path: cursor_marketplace_root.display().to_string(),
                },
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(added.marketplace.id, "cursor-local");

    let added_claude: MarketplacesAddResult = request(
        &client,
        "marketplaces/add",
        Some(
            serde_json::to_value(MarketplacesAddParams {
                id: "claude-local".to_string(),
                kind: Some(roder_api::marketplace::MarketplaceKind::Claude),
                display_name: "Claude Local".to_string(),
                source: roder_api::marketplace::MarketplaceSource::LocalPath {
                    path: claude_marketplace_root.display().to_string(),
                },
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(added_claude.marketplace.id, "claude-local");

    let refreshed: MarketplacesRefreshResult = request(
        &client,
        "marketplaces/refresh",
        Some(
            serde_json::to_value(MarketplacesRefreshParams {
                marketplace_id: "cursor-local".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(refreshed.plugins.len(), 1);
    assert_eq!(refreshed.plugins[0].plugin_id, "repo-tools");

    let refreshed_claude: MarketplacesRefreshResult = request(
        &client,
        "marketplaces/refresh",
        Some(
            serde_json::to_value(MarketplacesRefreshParams {
                marketplace_id: "claude-local".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(refreshed_claude.plugins.len(), 1);
    assert_eq!(refreshed_claude.plugins[0].plugin_id, "repo-tools-claude");

    let search: MarketplacesSearchResult = request(
        &client,
        "marketplaces/search",
        Some(
            serde_json::to_value(MarketplacesSearchParams {
                query: Some("repo".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(search.plugins.len(), 1);
    assert_eq!(search.plugins[0].variants.len(), 2);

    let preview: PluginPreviewInstallResult = request(
        &client,
        "plugins/preview_install",
        Some(
            serde_json::to_value(PluginPreviewInstallParams {
                marketplace_id: "cursor-local".to_string(),
                plugin_id: "repo-tools".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(preview.preview["displayName"], "Repo Tools");

    let installed: PluginInstallResult = request(
        &client,
        "plugins/install",
        Some(
            serde_json::to_value(PluginInstallParams {
                marketplace_id: "cursor-local".to_string(),
                plugin_id: "repo-tools".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(installed.plugin.variant_key, "cursor-local:repo-tools");

    let search_after_install: MarketplacesSearchResult = request(
        &client,
        "marketplaces/search",
        Some(
            serde_json::to_value(MarketplacesSearchParams {
                query: Some("repo".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(search_after_install.plugins.len(), 1);
    assert_eq!(search_after_install.plugins[0].variants.len(), 2);
    assert_eq!(
        search_after_install.plugins[0].installed_variants,
        vec!["cursor-local:repo-tools".to_string()]
    );

    let list: PluginListInstalledResult = request(&client, "plugins/list_installed", None).await;
    assert_eq!(list.plugins.len(), 1);
    assert!(cache_dir.join("cursor-local").join("repo-tools").exists());

    let workflow_scan: WorkflowScanResult = request(
        &client,
        "workflow/scan",
        Some(
            serde_json::to_value(WorkflowScanParams {
                workspace: Some(root.display().to_string()),
                include_user: true,
            })
            .unwrap(),
        ),
    )
    .await;
    let plugin_import = workflow_scan
        .scan
        .items
        .iter()
        .find(|item| item.source.name.as_deref() == Some("cursor-local:repo-tools"))
        .expect("installed marketplace plugin workflow import");
    assert_eq!(plugin_import.preview["marketplaceId"], "cursor-local");
    assert!(plugin_import.command_capable);
    assert!(plugin_import.approval_required);

    let installed_all: PluginInstallAllVariantsResult = request(
        &client,
        "plugins/install_all_variants",
        Some(
            serde_json::to_value(PluginInstallAllVariantsParams {
                marketplace_id: "cursor-local".to_string(),
                plugin_id: "repo-tools".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(installed_all.plugins.len(), 2);
    assert!(
        installed_all
            .plugins
            .iter()
            .any(|plugin| plugin.variant_key == "cursor-local:repo-tools")
    );
    assert!(
        installed_all
            .plugins
            .iter()
            .any(|plugin| plugin.variant_key == "claude-local:repo-tools-claude")
    );

    let disabled: PluginDisableResult = request(
        &client,
        "plugins/disable",
        Some(
            serde_json::to_value(PluginDisableParams {
                variant_key: "claude-local:repo-tools-claude".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        disabled.plugin.as_ref().map(|plugin| &plugin.state),
        Some(&MarketplaceInstallState::Disabled)
    );

    let uninstalled: PluginUninstallResult = request(
        &client,
        "plugins/uninstall",
        Some(
            serde_json::to_value(PluginUninstallParams {
                variant_key: "claude-local:repo-tools-claude".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(uninstalled.removed);

    let removed: MarketplacesRemoveResult = request(
        &client,
        "marketplaces/remove",
        Some(
            serde_json::to_value(MarketplacesRemoveParams {
                marketplace_id: "cursor-local".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(removed.removed);

    let marketplaces: MarketplacesListResult = request(&client, "marketplaces/list", None).await;
    assert!(
        !marketplaces
            .marketplaces
            .iter()
            .any(|marketplace| marketplace.id == "cursor-local")
    );
}

#[tokio::test]
async fn marketplace_methods_reject_invalid_sources_and_duplicate_plugins() {
    let root = std::env::temp_dir().join(format!(
        "roder-marketplace-validation-e2e-{}",
        uuid::Uuid::new_v4()
    ));
    let store_path = root.join("state").join("marketplaces.json");
    let cache_dir = root.join("cache");
    let marketplace_root = root.join("cursor-marketplace");
    std::fs::create_dir_all(marketplace_root.join(".cursor-plugin")).unwrap();
    std::fs::write(
        marketplace_root
            .join(".cursor-plugin")
            .join("marketplace.json"),
        serde_json::json!({
            "plugins": [
                {
                    "id": "dupe-tools",
                    "name": "Dupe Tools",
                    "repository": "https://github.com/example/dupe-tools",
                    "source": "dupe-tools-a"
                },
                {
                    "id": "dupe-tools",
                    "name": "Dupe Tools Copy",
                    "repository": "https://github.com/example/dupe-tools-copy",
                    "source": "dupe-tools-b"
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    unsafe {
        std::env::set_var("RODER_MARKETPLACES_PATH", &store_path);
        std::env::set_var("RODER_MARKETPLACE_CACHE_DIR", &cache_dir);
    }

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let invalid = request_error(
        &client,
        "marketplaces/add",
        Some(
            serde_json::to_value(MarketplacesAddParams {
                id: "bad source".to_string(),
                kind: Some(roder_api::marketplace::MarketplaceKind::Cursor),
                display_name: "Bad Source".to_string(),
                source: roder_api::marketplace::MarketplaceSource::Git {
                    url: "ftp://example.test/plugins.git".to_string(),
                    ref_name: None,
                    catalog_path: None,
                },
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(invalid.code, -32602);

    let added: MarketplacesAddResult = request(
        &client,
        "marketplaces/add",
        Some(
            serde_json::to_value(MarketplacesAddParams {
                id: "cursor-validation".to_string(),
                kind: Some(roder_api::marketplace::MarketplaceKind::Cursor),
                display_name: "Cursor Validation".to_string(),
                source: roder_api::marketplace::MarketplaceSource::LocalPath {
                    path: marketplace_root.display().to_string(),
                },
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(added.marketplace.id, "cursor-validation");

    let duplicate_marketplace = request_error(
        &client,
        "marketplaces/add",
        Some(
            serde_json::to_value(MarketplacesAddParams {
                id: "cursor-validation".to_string(),
                kind: Some(roder_api::marketplace::MarketplaceKind::Cursor),
                display_name: "Cursor Validation Duplicate".to_string(),
                source: roder_api::marketplace::MarketplaceSource::LocalPath {
                    path: marketplace_root.display().to_string(),
                },
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(duplicate_marketplace.code, -32602);
    assert!(
        duplicate_marketplace
            .message
            .contains("duplicate marketplace")
    );

    let duplicate_plugin = request_error(
        &client,
        "marketplaces/refresh",
        Some(
            serde_json::to_value(MarketplacesRefreshParams {
                marketplace_id: "cursor-validation".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(duplicate_plugin.code, -32000);
    assert!(duplicate_plugin.message.contains("duplicate plugin"));
}

#[tokio::test]
async fn media_methods_read_thumbnail_attach_and_delete_artifacts() {
    let root = std::env::temp_dir().join(format!("roder-media-e2e-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("RODER_MEDIA_ARTIFACT_DIR", &root);
    }
    let store = MediaArtifactStore::new(&root);
    let (artifact, _) = store
        .write_generated(
            &MediaGenerationRequest {
                prompt: "attach me".to_string(),
                model: None,
                output_path: None,
            },
            MediaKind::Image,
            "image/png",
            "fake",
            b"abc",
            Some(MediaDimensions {
                width: 1,
                height: 1,
            }),
            None,
        )
        .unwrap();

    let runtime = Arc::new(
        Runtime::new(
            build_default_registry(DefaultRegistryConfig::default()).unwrap(),
            Default::default(),
        )
        .unwrap(),
    );
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let listed: MediaListResult = request(
        &client,
        "media/list",
        Some(
            serde_json::to_value(MediaListParams {
                thread_id: None,
                kind: Some(MediaKind::Image),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(listed.artifacts.len(), 1);

    let read: MediaReadResult = request(
        &client,
        "media/read",
        Some(
            serde_json::to_value(MediaReadParams {
                artifact_id: artifact.id.clone(),
                max_bytes: Some(1024),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(read.bytes_base64, "YWJj");

    let thumbnail: MediaThumbnailResult = request(
        &client,
        "media/thumbnail",
        Some(
            serde_json::to_value(MediaThumbnailParams {
                artifact_id: artifact.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(thumbnail.preview.artifact_id, artifact.id);

    let attach: MediaAttachToTurnResult = request(
        &client,
        "media/attachToTurn",
        Some(
            serde_json::to_value(MediaAttachToTurnParams {
                artifact_id: artifact.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(attach.attachment.data_url, "data:image/png;base64,YWJj");
    assert_eq!(
        attach.image.unwrap().image_url,
        "data:image/png;base64,YWJj"
    );

    let deleted: MediaDeleteResult = request(
        &client,
        "media/delete",
        Some(
            serde_json::to_value(MediaDeleteParams {
                artifact_id: artifact.id,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(deleted.deleted);
}

#[tokio::test]
async fn memory_methods_save_query_read_update_delete_and_preview() {
    let root = std::env::temp_dir().join(format!("roder-memory-e2e-{}", uuid::Uuid::new_v4()));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.install(MemoryExtension::new(root)).unwrap();
    builder
        .install(OpenAiEmbeddingsExtension::with_api_key("test-key"))
        .unwrap();
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let saved: MemorySaveResult = request(
        &client,
        "memory/save",
        Some(
            serde_json::to_value(MemorySaveParams {
                scope: MemoryScope::Project("gode".to_string()),
                text: "sqlite vector memories recall project facts".to_string(),
                metadata: serde_json::json!({"source":"test"}),
            })
            .unwrap(),
        ),
    )
    .await;

    let listed: MemoryListResult = request(
        &client,
        "memory/list",
        Some(
            serde_json::to_value(MemoryListParams {
                scope: Some(MemoryScope::Project("gode".to_string())),
                limit: Some(10),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(listed.memories.len(), 1);

    let queried: MemoryQueryResult = request(
        &client,
        "memory/query",
        Some(
            serde_json::to_value(MemoryQueryParams {
                scope: Some(MemoryScope::Project("gode".to_string())),
                text: "vector recall".to_string(),
                limit: Some(5),
                include_global: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        queried.results[0].record.id.as_deref(),
        Some(saved.memory_id.as_str())
    );
    assert!(queried.results[0].citation.is_some());

    let read: MemoryReadResult = request(
        &client,
        "memory/read",
        Some(
            serde_json::to_value(MemoryReadParams {
                memory_id: saved.memory_id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(read.memory.unwrap().text.contains("sqlite vector"));

    let updated: MemorySaveResult = request(
        &client,
        "memory/update",
        Some(
            serde_json::to_value(MemoryUpdateParams {
                memory_id: saved.memory_id.clone(),
                text: "updated memory fact".to_string(),
                metadata: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(updated.memory_id, saved.memory_id);

    let providers: MemoryProviderListResult = request(&client, "memory/provider/list", None).await;
    assert!(
        providers
            .providers
            .iter()
            .any(|provider| provider.id == "openai")
    );

    let preview: MemoryRecallPreviewResult = request(
        &client,
        "memory/recall/preview",
        Some(
            serde_json::to_value(MemoryRecallPreviewParams {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                scope: Some(MemoryScope::Project("gode".to_string())),
                text: "updated".to_string(),
                limit: Some(5),
                include_global: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!preview.citations.is_empty());

    let deleted: MemoryDeleteResult = request(
        &client,
        "memory/delete",
        Some(
            serde_json::to_value(MemoryDeleteParams {
                memory_id: saved.memory_id,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(deleted.deleted);
}

#[tokio::test]
async fn remote_websocket_requires_auth_and_serves_thread_turn_flow() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let mut events = runtime.subscribe_events();
    let app_server = Arc::new(AppServer::new(runtime));
    let token = RemoteToken::new("remote-secret-token".to_string()).unwrap();
    let handle = listen_remote_websocket(
        app_server,
        RemoteServerOptions {
            listen: "ws://127.0.0.1:0".to_string(),
            token,
            token_ttl: None,
            allowed_origins: Vec::new(),
            print_qr: false,
            workspace: Some("/tmp/gode".to_string()),
        },
    )
    .await
    .unwrap();
    let started = wait_for_global_event(&mut events, "remote/serverStarted").await;
    assert!(
        !serde_json::to_string(&started.event)
            .unwrap()
            .contains("remote-secret-token")
    );
    let url = format!("ws://{}", handle.listen_addr);

    let rejected = tokio_tungstenite::connect_async(&url).await;
    assert!(rejected.is_err());
    let auth_failed = wait_for_global_event(&mut events, "remote/authFailed").await;
    assert!(
        !serde_json::to_string(&auth_failed.event)
            .unwrap()
            .contains("remote-secret-token")
    );

    let mut origin_request = url.clone().into_client_request().unwrap();
    origin_request.headers_mut().insert(
        "Authorization",
        "Bearer remote-secret-token".parse().unwrap(),
    );
    origin_request
        .headers_mut()
        .insert("Origin", "https://client.example".parse().unwrap());
    assert!(
        tokio_tungstenite::connect_async(origin_request)
            .await
            .is_err()
    );

    let mut request = url.clone().into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        "Bearer remote-secret-token".parse().unwrap(),
    );
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let connected = wait_for_global_event(&mut events, "remote/clientConnected").await;
    assert_eq!(connected.source, roder_api::events::EventSource::AppServer);
    websocket
        .send(Message::Text(
            serde_json::to_string(&JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("init")),
                method: "initialize".to_string(),
                params: None,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let message = websocket.next().await.unwrap().unwrap();
    let Message::Text(text) = message else {
        panic!("expected text response");
    };
    let response: roder_protocol::JsonRpcResponse = serde_json::from_str(&text).unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    assert_eq!(
        result
            .get("remote")
            .and_then(|remote| remote.get("authenticated"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let started: ThreadStartResult = remote_request(
        &mut websocket,
        "thread-start",
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": "/tmp",
            "ephemeral": false
        })),
    )
    .await;
    let thread_id = started.thread.id.clone();

    let turn: TurnStartResult = remote_request(
        &mut websocket,
        "turn-start",
        "turn/start",
        Some(serde_json::json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": "hello remote" }]
        })),
    )
    .await;
    assert!(!turn.turn_id.is_empty());

    let mut saw_completed = false;
    for _ in 0..20 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(&started.thread.id)
            && envelope.kind == "turn.completed"
        {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "remote turn did not complete");

    let read: ThreadReadResult = remote_request(
        &mut websocket,
        "thread-read",
        "thread/read",
        Some(serde_json::json!({
            "threadId": started.thread.id,
            "includeTurns": true
        })),
    )
    .await;
    let thread = read.thread.expect("remote thread/read returns thread");
    assert_eq!(thread.id, started.thread.id);

    drop(websocket);
    let _ = wait_for_global_event(&mut events, "remote/clientDisconnected").await;
}

#[tokio::test]
async fn remote_server_controller_stop_emits_stopped_event() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let mut events = runtime.subscribe_events();
    let app_server = Arc::new(AppServer::new(runtime));
    let token = RemoteToken::new("remote-secret-token".to_string()).unwrap();

    let controller = listen_remote_websocket_controller(
        app_server,
        RemoteServerOptions {
            listen: "ws://127.0.0.1:0".to_string(),
            token,
            token_ttl: None,
            allowed_origins: Vec::new(),
            print_qr: false,
            workspace: Some("/tmp/gode".to_string()),
        },
    )
    .await
    .unwrap();
    let listen_addr = controller.handle().listen_addr.to_string();

    let started = wait_for_global_event(&mut events, "remote/serverStarted").await;
    assert_eq!(started.source, roder_api::events::EventSource::AppServer);

    controller.stop().await.unwrap();

    let stopped = wait_for_global_event(&mut events, "remote/serverStopped").await;
    assert_eq!(stopped.source, roder_api::events::EventSource::AppServer);
    assert!(
        serde_json::to_string(&stopped.event)
            .unwrap()
            .contains(&listen_addr)
    );
}

#[tokio::test]
async fn remote_health_endpoints_do_not_require_auth() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let app_server = Arc::new(AppServer::new(runtime));
    let token = RemoteToken::new("remote-secret-token".to_string()).unwrap();

    let controller = listen_remote_websocket_controller(
        app_server,
        RemoteServerOptions {
            listen: "ws://127.0.0.1:0".to_string(),
            token,
            token_ttl: None,
            allowed_origins: Vec::new(),
            print_qr: false,
            workspace: Some("/tmp/gode".to_string()),
        },
    )
    .await
    .unwrap();

    for path in ["/readyz", "/healthz"] {
        let mut stream = TcpStream::connect(controller.handle().listen_addr)
            .await
            .unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: roder\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buffer = [0_u8; 512];
        let bytes_read = stream.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..bytes_read]);

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "unexpected {path} response: {response:?}"
        );
        assert!(response.ends_with("\r\n\r\nok\n"));
    }

    controller.stop().await.unwrap();
}

#[tokio::test]
async fn providers_list_exposes_xai_and_supergrok_auth_metadata() {
    let registry = build_default_registry(DefaultRegistryConfig {
        xai_api_key: Some("secret-xai-key".to_string()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let xai = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_XAI)
        .expect("xai provider should be listed when an API key is configured");
    assert_eq!(xai.auth_type, ProviderAuthType::ApiKey);
    assert!(xai.authenticated);
    assert!(xai.models.iter().any(|model| model.id == "grok-4.3"));

    let supergrok = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_SUPERGROK)
        .expect("supergrok provider should be listed");
    assert_eq!(supergrok.auth_type, ProviderAuthType::OAuth);
    assert!(
        supergrok
            .models
            .iter()
            .any(|model| model.id == "grok-4.20-0309-reasoning")
    );
}

#[tokio::test]
async fn providers_select_opencode_non_reasoning_model_preserves_reasoning_preference() {
    let registry = build_default_registry(DefaultRegistryConfig {
        opencode_api_key: Some("secret-opencode-key".to_string()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(
        Runtime::new(
            registry,
            RuntimeConfig {
                reasoning: Some("high".to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let selected: ProviderSelectResult = request(
        &client,
        "providers/select",
        Some(
            serde_json::to_value(ProviderSelectParams {
                provider: PROVIDER_OPENCODE.to_string(),
                model: Some("big-pickle".to_string()),
                reasoning: Some("none".to_string()),
                thread_id: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(selected.provider, PROVIDER_OPENCODE);
    assert_eq!(selected.model, "big-pickle");
    assert_eq!(selected.reasoning, "none");

    let selected: ProviderSelectResult = request(
        &client,
        "providers/select",
        Some(
            serde_json::to_value(ProviderSelectParams {
                provider: PROVIDER_CODEX.to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: None,
                thread_id: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(selected.provider, PROVIDER_CODEX);
    assert_eq!(selected.model, "gpt-5.5");
    assert_eq!(selected.reasoning, "high");
}

#[tokio::test]
async fn providers_list_separates_opencode_zen_and_go_models() {
    let cache_path = std::env::temp_dir().join(format!(
        "roder-opencode-provider-list-e2e-{}.json",
        uuid::Uuid::new_v4()
    ));
    let _models_cache = EnvVarGuard::set("RODER_MODELS_CACHE_PATH", &cache_path);

    let registry = build_default_registry(DefaultRegistryConfig {
        opencode_api_key: Some("secret-opencode-key".to_string()),
        opencode_go_api_key: Some("secret-opencode-go-key".to_string()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let zen = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_OPENCODE)
        .expect("opencode provider should be listed");
    assert!(zen.models.iter().any(|model| model.id == "big-pickle"));
    assert!(!zen.models.iter().any(|model| model.id == "kimi-k2.6"));

    let go = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_OPENCODE_GO)
        .expect("opencode-go provider should be listed");
    assert!(go.models.iter().any(|model| model.id == "kimi-k2.6"));
    assert!(!go.models.iter().any(|model| model.id == "big-pickle"));
}

#[tokio::test]
async fn providers_list_exposes_poolside_api_key_models() {
    let registry = build_default_registry(DefaultRegistryConfig {
        poolside_api_key: Some("secret-poolside-key".to_string()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let poolside = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_POOLSIDE)
        .expect("poolside provider should be listed");
    assert_eq!(poolside.auth_type, ProviderAuthType::ApiKey);
    assert!(poolside.authenticated);
    assert!(
        poolside
            .models
            .iter()
            .any(|model| model.id == "poolside/laguna-m.1")
    );
}

#[tokio::test]
async fn providers_list_exposes_cursor_api_key_models() {
    let registry = build_default_registry(DefaultRegistryConfig {
        cursor_api_key: Some("secret-cursor-key".to_string()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let cursor = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_CURSOR)
        .expect("cursor provider should be listed");
    assert_eq!(cursor.auth_type, ProviderAuthType::ApiKey);
    assert!(cursor.authenticated);
    assert!(cursor.models.iter().any(|model| model.id == "composer-2.5"));
    assert!(cursor.capabilities.tool_calls);
    assert!(!cursor.capabilities.structured_output);
}

#[tokio::test]
async fn providers_clear_removes_api_key() {
    let temp_dir = std::env::temp_dir().join(format!("roder-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    unsafe {
        std::env::set_var("RODER_CONFIG_DIR", &temp_dir);
    }

    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = AppServer::new(runtime).with_user_config_persistence();
    let client = LocalAppClient::new(Arc::new(server));

    // Initially cursor is not authenticated
    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let cursor = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_CURSOR)
        .expect("cursor provider should be listed");
    assert!(!cursor.authenticated);

    // Configure it
    let configure_params = ProviderConfigureParams {
        provider: PROVIDER_CURSOR.to_string(),
        api_key: "secret-cursor-key".to_string(),
    };
    let configure_res: ProviderConfigureResult = request(
        &client,
        "providers/configure",
        Some(serde_json::to_value(configure_params).unwrap()),
    )
    .await;
    assert!(configure_res.authenticated);

    // Check providers list again to verify authenticated is true
    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let cursor = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_CURSOR)
        .expect("cursor provider should be listed");
    assert!(cursor.authenticated);

    // Clear it
    let clear_params = ProviderClearParams {
        provider: PROVIDER_CURSOR.to_string(),
    };
    let clear_res: ProviderClearResult = request(
        &client,
        "providers/clear",
        Some(serde_json::to_value(clear_params).unwrap()),
    )
    .await;
    assert_eq!(clear_res.provider, PROVIDER_CURSOR);

    // Check providers list again to verify authenticated is false
    let providers: ProvidersListResult = request(&client, "providers/list", None).await;
    let cursor = providers
        .providers
        .iter()
        .find(|provider| provider.id == PROVIDER_CURSOR)
        .expect("cursor provider should be listed");
    assert!(!cursor.authenticated);

    // Clean up temp dir
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn supergrok_auth_status_is_exposed_through_app_server() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let status: ProviderAuthResult = request(&client, "auth/supergrok/status", None).await;
    if !status.signed_in {
        assert_eq!(status.account_id, None);
    }
}

#[tokio::test]
async fn runners_methods_list_select_status_and_delete_destination() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let listed: RunnersListResult = request(&client, "runners/list", None).await;
    assert!(
        listed
            .providers
            .iter()
            .any(|provider| provider.provider_id == "unix-local")
    );

    let selected: RunnersSelectResult = request(
        &client,
        "runners/select",
        Some(
            serde_json::to_value(RunnersSelectParams {
                destination_id: "unix-local".to_string(),
                provider_id: Some("unix-local".to_string()),
                config: serde_json::Value::Null,
                manifest: roder_api::remote_runner::RunnerManifest::default(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        selected.active.as_ref().map(|runner| runner.state.as_str()),
        Some("configured")
    );
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.kind, "runner.lifecycle");

    let session: RunnersSessionResult = request(&client, "runners/session", None).await;
    assert_eq!(
        session
            .active
            .as_ref()
            .map(|runner| runner.destination_id.as_str()),
        Some("unix-local")
    );

    let deleted: RunnersDeleteResult = request(&client, "runners/delete", None).await;
    assert!(deleted.deleted);
    let session: RunnersSessionResult = request(&client, "runners/session", None).await;
    assert!(session.active.is_none());

    let missing_secret = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("runner-missing-secret")),
            method: "runners/select".to_string(),
            params: Some(
                serde_json::to_value(RunnersSelectParams {
                    destination_id: "blaxel".to_string(),
                    provider_id: Some("blaxel".to_string()),
                    config: serde_json::Value::Null,
                    manifest: roder_api::remote_runner::RunnerManifest::default(),
                })
                .unwrap(),
            ),
        })
        .await;
    let missing_error = missing_secret.error.expect("missing token should fail");
    assert_eq!(missing_error.code, -32602);
    assert!(missing_error.message.contains("BLAXEL_API_KEY"));
    assert!(!missing_error.message.contains("plain-token"));

    let hosted: RunnersSelectResult = request(
        &client,
        "runners/select",
        Some(
            serde_json::to_value(RunnersSelectParams {
                destination_id: "blaxel".to_string(),
                provider_id: Some("blaxel".to_string()),
                config: serde_json::json!({ "token": "plain-token" }),
                manifest: roder_api::remote_runner::RunnerManifest::default(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        hosted
            .active
            .as_ref()
            .map(|runner| runner.provider_id.as_str()),
        Some("blaxel")
    );
    let session: RunnersSessionResult = request(&client, "runners/session", None).await;
    let encoded_session = serde_json::to_string(&session).unwrap();
    assert!(!encoded_session.contains("plain-token"));
}

#[tokio::test]
async fn internal_errors_include_structured_details() {
    let engine = Arc::new(FakeInferenceEngine);
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(FailingThreadStoreFactory));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/list")),
            method: "thread/list".to_string(),
            params: Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
        })
        .await;

    let error = response.error.expect("missing internal error");

    assert_eq!(error.code, -32000);
    assert!(error.message.contains("parse thread metadata"));
    assert!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("details"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|details| details.contains("metadata.json"))
    );
}

#[tokio::test]
async fn turn_steer_requires_active_turn() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("turn/steer")),
            method: "turn/steer".to_string(),
            params: Some(
                serde_json::to_value(TurnSteerParams {
                    thread_id: "thread_missing".to_string(),
                    expected_turn_id: "turn_missing".to_string(),
                    input: text_input("change direction"),
                    prompt: None,
                })
                .unwrap(),
            ),
        })
        .await;

    assert!(response.result.is_none());
    let error = response.error.expect("missing steer error");
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "no active turn to steer");
}

#[tokio::test]
async fn turn_steer_accepts_active_turn() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let thread_start = start_thread(&client).await;
    let started = start_turn(&client, &thread_start.thread.id, "start").await;
    wait_for_event(&mut events, &thread_start.thread.id, "turn.started").await;

    let steered: TurnSteerResult = request(
        &client,
        "turn/steer",
        Some(
            serde_json::to_value(TurnSteerParams {
                thread_id: thread_start.thread.id.clone(),
                expected_turn_id: started.turn_id.clone(),
                input: text_input("change direction"),
                prompt: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(steered.turn_id, started.turn_id);

    let event = wait_for_event(&mut events, &thread_start.thread.id, "turn.steered").await;
    assert_eq!(event.turn_id.as_deref(), Some(started.turn_id.as_str()));

    let _: roder_protocol::TurnInterruptResult = request(
        &client,
        "turn/interrupt",
        Some(
            serde_json::to_value(TurnInterruptParams {
                thread_id: thread_start.thread.id,
                turn_id: Some(started.turn_id),
            })
            .unwrap(),
        ),
    )
    .await;
}

#[tokio::test]
async fn protocol_contract_methods_support_protocol_startup_contract() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let initialize: serde_json::Value =
        request(&client, "initialize", Some(serde_json::json!({}))).await;
    assert_eq!(initialize["provider"], PROVIDER_MOCK);

    let models: serde_json::Value =
        request(&client, "model/list", Some(serde_json::json!({}))).await;
    assert!(
        models["models"]
            .as_array()
            .is_some_and(|models| !models.is_empty())
    );

    let list: serde_json::Value = request(
        &client,
        "thread/list",
        Some(serde_json::json!({ "limit": 100 })),
    )
    .await;
    assert!(list["data"].as_array().is_some());

    let started: serde_json::Value = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": "/tmp",
            "ephemeral": false
        })),
    )
    .await;
    let thread_id = started["thread"]["id"]
        .as_str()
        .expect("thread/start returns thread.id")
        .to_string();

    let read: serde_json::Value = request(
        &client,
        "thread/read",
        Some(serde_json::json!({
            "threadId": thread_id,
            "includeTurns": true
        })),
    )
    .await;
    assert_eq!(read["thread"]["id"], started["thread"]["id"]);
}

#[tokio::test]
async fn thread_archive_removes_thread_from_protocol_thread_list() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(RecordingThreadStoreFactory::default()));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let started = start_thread(&client).await;

    let archived: ThreadArchiveResult = request(
        &client,
        "thread/archive",
        Some(
            serde_json::to_value(ThreadArchiveParams {
                thread_id: started.thread.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(archived.archived);
    assert_eq!(archived.thread_id, started.thread.id);

    let threads: ThreadListResult = request(
        &client,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
    )
    .await;
    assert!(
        !threads
            .data
            .iter()
            .any(|thread| thread.id == archived.thread_id)
    );
}

#[tokio::test]
async fn protocol_contract_turn_methods_and_notifications_match_protocol_contract() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let started: serde_json::Value = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": "/tmp",
            "ephemeral": false
        })),
    )
    .await;
    let thread_id = started["thread"]["id"]
        .as_str()
        .expect("thread/start returns thread.id")
        .to_string();
    let thread_started =
        wait_for_notification(&mut notifications, "thread/started", Some(&thread_id)).await;
    assert_eq!(thread_started.params["thread"]["id"], thread_id);
    assert_eq!(thread_started.params["thread"]["status"]["type"], "idle");

    let turn: serde_json::Value = request(
        &client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": "hello" }]
        })),
    )
    .await;
    let turn_id = turn["turnId"]
        .as_str()
        .expect("turn/start returns turnId")
        .to_string();

    let mut methods = Vec::new();
    let mut saw_delta = false;
    let mut saw_item_completed = false;
    let mut saw_status_active = false;
    for _ in 0..20 {
        let notification = tokio::time::timeout(Duration::from_secs(2), notifications.recv())
            .await
            .unwrap()
            .unwrap();
        methods.push(notification.method.clone());
        match notification.method.as_str() {
            "turn/started" => {
                assert_eq!(notification.params["threadId"], thread_id);
                assert_eq!(notification.params["turn"]["id"], turn_id);
                assert_eq!(notification.params["turn"]["status"], "inProgress");
            }
            "item/agentMessage/delta" => {
                assert_eq!(notification.params["threadId"], thread_id);
                assert_eq!(notification.params["turnId"], turn_id);
                assert!(notification.params["itemId"].as_str().is_some());
                assert!(notification.params["delta"].as_str().is_some());
                saw_delta = true;
            }
            "item/completed" => {
                assert_eq!(notification.params["threadId"], thread_id);
                assert_eq!(notification.params["turnId"], turn_id);
                assert!(notification.params["item"]["type"].as_str().is_some());
                saw_item_completed = true;
            }
            "thread/status/changed" if notification.params["status"]["type"] == "running" => {
                saw_status_active = true;
            }
            "turn/completed" => {
                assert_eq!(notification.params["threadId"], thread_id);
                assert_eq!(notification.params["turn"]["id"], turn_id);
                assert!(notification.params["turn"]["status"].as_str().is_some());
                break;
            }
            _ => {}
        }
    }

    assert!(methods.iter().any(|method| method == "turn/started"));
    assert!(saw_delta, "missing agent delta notification: {methods:?}");
    assert!(
        saw_item_completed,
        "missing item completion notification: {methods:?}"
    );
    assert!(
        saw_status_active,
        "missing active status notification: {methods:?}"
    );
    assert!(methods.iter().any(|method| method == "turn/completed"));
}

#[tokio::test]
async fn turn_usage_cache_metrics_are_exposed_on_notifications_and_thread_metadata() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(UsageReportingEngine));
    builder.thread_store_factory(Arc::new(RecordingThreadStoreFactory::default()));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();
    let thread = start_thread(&client).await;
    let thread_id = thread.thread.id.clone();
    let turn = start_turn(&client, &thread_id, "record usage").await;

    let completed =
        wait_for_notification(&mut notifications, "turn/completed", Some(&thread_id)).await;

    assert_eq!(completed.params["turn"]["id"], turn.turn_id);
    assert_eq!(completed.params["turn"]["usage"]["prompt_tokens"], 100);
    assert_eq!(
        completed.params["turn"]["usage"]["cached_prompt_tokens"],
        92
    );
    assert!(
        (completed.params["turn"]["usage"]["cache_hit_rate"]
            .as_f64()
            .unwrap()
            - 0.92)
            .abs()
            < f64::EPSILON
    );

    let read: ThreadReadResult = request(
        &client,
        "thread/read",
        Some(
            serde_json::to_value(ThreadReadParams {
                thread_id,
                include_turns: true,
            })
            .unwrap(),
        ),
    )
    .await;
    let thread = read.thread.expect("thread/read returns persisted thread");
    let usage = thread.usage.expect("thread metadata includes usage");
    assert_eq!(usage.prompt_tokens, 100);
    assert_eq!(usage.cached_prompt_tokens, 92);
    assert!((usage.cache_hit_rate.unwrap() - 0.92).abs() < f64::EPSILON);
    assert_eq!(
        thread.turns.unwrap()[0]
            .usage
            .as_ref()
            .unwrap()
            .cached_prompt_tokens,
        92
    );
}

#[tokio::test]
async fn protocol_contract_turn_interrupt_uses_active_turn_when_turn_id_is_omitted() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let started: serde_json::Value = request(
        &client,
        "thread/start",
        Some(serde_json::json!({
            "model": "mock",
            "modelProvider": PROVIDER_MOCK,
            "cwd": "/tmp",
            "ephemeral": false
        })),
    )
    .await;
    let thread_id = started["thread"]["id"]
        .as_str()
        .expect("thread/start returns thread.id")
        .to_string();

    let turn: serde_json::Value = request(
        &client,
        "turn/start",
        Some(serde_json::json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": "wait" }]
        })),
    )
    .await;
    let turn_id = turn["turnId"]
        .as_str()
        .expect("turn/start returns turnId")
        .to_string();

    let steered: serde_json::Value = request(
        &client,
        "turn/steer",
        Some(serde_json::json!({
            "threadId": thread_id,
            "expectedTurnId": turn_id,
            "input": [{ "type": "text", "text": "follow up" }]
        })),
    )
    .await;
    assert_eq!(steered["turnId"].as_str(), Some(turn_id.as_str()));

    let interrupted: serde_json::Value = request(
        &client,
        "turn/interrupt",
        Some(serde_json::json!({
            "threadId": thread_id
        })),
    )
    .await;
    assert_eq!(interrupted["turnId"], turn_id);
}

#[tokio::test]
async fn protocol_notifications_surface_tool_approval_requests_and_resolution() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ApprovalRequiredEngine {
        calls: Mutex::new(0),
    }));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(".").unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let thread_start = start_thread(&client).await;
    let _started = start_turn(&client, &thread_start.thread.id, "what branch are you on?").await;

    let approval = wait_for_notification(
        &mut notifications,
        "thread/approvalRequested",
        Some(&thread_start.thread.id),
    )
    .await;
    assert_eq!(approval.params["turnId"].as_str().is_some(), true);
    assert_eq!(approval.params["approvalId"], "approval-shell-1");
    assert_eq!(approval.params["toolId"], "approval-shell-1");
    assert_eq!(approval.params["toolName"], "shell");

    let waiting_status = wait_for_notification(
        &mut notifications,
        "thread/status/changed",
        Some(&thread_start.thread.id),
    )
    .await;
    assert_eq!(waiting_status.params["status"]["type"], "running");
    assert_eq!(
        waiting_status.params["status"]["activeFlags"],
        serde_json::json!(["approvalRequired"])
    );

    let resolved: ThreadResolveApprovalResult = request(
        &client,
        "thread/resolve_approval",
        Some(
            serde_json::to_value(ThreadResolveApprovalParams {
                approval_id: "approval-shell-1".to_string(),
                approved: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(resolved.resolved);

    let resolved_notification = wait_for_notification(
        &mut notifications,
        "thread/approvalResolved",
        Some(&thread_start.thread.id),
    )
    .await;
    assert_eq!(
        resolved_notification.params["approvalId"],
        "approval-shell-1"
    );
    assert_eq!(resolved_notification.params["approved"], false);
}

#[tokio::test]
async fn protocol_contract_fs_and_command_methods_match_protocol_contract() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let dir = std::env::temp_dir().join(format!("roder-fs-command-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hello.txt");
    std::fs::write(&file, b"hello fs").unwrap();

    let read_file: serde_json::Value = request(
        &client,
        "fs/readFile",
        Some(serde_json::json!({ "path": file.display().to_string() })),
    )
    .await;
    let data = base64::engine::general_purpose::STANDARD
        .decode(read_file["dataBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(data, b"hello fs");

    let read_dir: serde_json::Value = request(
        &client,
        "fs/readDirectory",
        Some(serde_json::json!({ "path": dir.display().to_string() })),
    )
    .await;
    assert!(
        read_dir["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["fileName"] == "hello.txt"
                && entry["isFile"] == true
                && entry["isDirectory"] == false)
    );

    let blocked = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("command/exec")),
            method: "command/exec".to_string(),
            params: Some(serde_json::json!({
                "command": ["sh", "-c", "printf blocked"],
                "cwd": dir.display().to_string(),
                "timeoutMs": 5000
            })),
        })
        .await;
    let blocked_error = blocked.error.expect("command/exec should be policy-gated");
    assert_eq!(blocked_error.code, -32004);
    assert_eq!(
        blocked_error.data.unwrap()["kind"],
        serde_json::json!("approval_required")
    );

    runtime
        .set_policy_mode(PolicyMode::AcceptAll, Some("test command/exec".to_string()))
        .await
        .unwrap();

    let command: serde_json::Value = request(
        &client,
        "command/exec",
        Some(serde_json::json!({
            "command": ["sh", "-c", "printf stdout; printf stderr >&2"],
            "cwd": dir.display().to_string(),
            "timeoutMs": 5000
        })),
    )
    .await;
    assert_eq!(command["exitCode"], 0);
    assert_eq!(command["stdout"], "stdout");
    assert_eq!(command["stderr"], "stderr");

    let streamed: serde_json::Value = request(
        &client,
        "command/exec",
        Some(serde_json::json!({
            "command": ["sh", "-c", "printf stream-out; printf stream-err >&2"],
            "cwd": dir.display().to_string(),
            "processId": "process-1",
            "streamStdoutStderr": true,
            "timeoutMs": 5000
        })),
    )
    .await;
    assert_eq!(streamed["exitCode"], 0);
    assert_eq!(streamed["stdout"], "");
    assert_eq!(streamed["stderr"], "");

    let stdout_delta =
        wait_for_notification(&mut notifications, "command/exec/outputDelta", None).await;
    assert_eq!(stdout_delta.params["processId"], "process-1");
    assert_eq!(stdout_delta.params["stream"], "stdout");
    let stdout = base64::engine::general_purpose::STANDARD
        .decode(stdout_delta.params["deltaBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(stdout, b"stream-out");

    let unsupported = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("command/exec/write")),
            method: "command/exec/write".to_string(),
            params: Some(serde_json::json!({ "processId": "process-1" })),
        })
        .await;
    assert_eq!(unsupported.error.unwrap().code, -32601);
}

#[tokio::test]
async fn artifacts_methods_list_read_grep_tail_delete_and_command_spill() {
    let data_dir =
        std::env::temp_dir().join(format!("roder-artifact-e2e-{}", uuid::Uuid::new_v4()));
    let thread_root = data_dir.join("threads");
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                team_data_dir: Some(data_dir.join("teams")),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    runtime
        .set_policy_mode(PolicyMode::AcceptAll, Some("test artifacts".to_string()))
        .await
        .unwrap();
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);

    let dir = std::env::temp_dir().join(format!("roder-command-artifact-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let command: serde_json::Value = request(
        &client,
        "command/exec",
        Some(serde_json::json!({
            "command": ["sh", "-c", "printf 'alpha\nneedle\nomega\n'"],
            "cwd": dir.display().to_string(),
            "processId": "process-artifact-1",
            "outputBytesCap": 8,
            "timeoutMs": 5000
        })),
    )
    .await;
    assert_eq!(command["exitCode"], 0);
    assert!(
        command["stdout"]
            .as_str()
            .unwrap()
            .contains("read_artifact")
    );
    let artifact_id = command["stdoutArtifact"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(command["stdoutArtifact"]["kind"], "command_stdout");
    let artifact_path = runtime
        .context_artifacts()
        .get(&artifact_id)
        .unwrap()
        .store_path;
    assert!(
        artifact_path.starts_with(
            thread_root
                .join("app-server")
                .join("artifacts")
                .join("process-artifact-1")
                .to_string_lossy()
                .as_ref()
        )
    );

    let listed: ArtifactListResult = request(
        &client,
        "artifact/list",
        Some(
            serde_json::to_value(ArtifactListParams {
                thread_id: "app-server".to_string(),
                kind: Some(ContextArtifactKind::CommandStdout),
                limit: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(
        listed
            .artifacts
            .iter()
            .any(|artifact| artifact.id == artifact_id)
    );

    let read: ArtifactReadResult = request(
        &client,
        "artifact/read",
        Some(
            serde_json::to_value(ArtifactReadParams {
                thread_id: "app-server".to_string(),
                artifact_id: artifact_id.clone(),
                start_line: Some(2),
                limit: Some(1),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(read.page.text.contains("2: needle"));

    let grep: ArtifactGrepResult = request(
        &client,
        "artifact/grep",
        Some(
            serde_json::to_value(ArtifactGrepParams {
                thread_id: "app-server".to_string(),
                artifact_id: artifact_id.clone(),
                query: "needle".to_string(),
                offset: None,
                limit: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(grep.page.total_matches, 1);

    let tail: ArtifactTailResult = request(
        &client,
        "artifact/tail",
        Some(
            serde_json::to_value(ArtifactTailParams {
                thread_id: "app-server".to_string(),
                artifact_id: artifact_id.clone(),
                lines: Some(1),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(tail.page.text.contains("3: omega"));

    let denied = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("artifact/read-denied")),
            method: "artifact/read".to_string(),
            params: Some(
                serde_json::to_value(ArtifactReadParams {
                    thread_id: "other-thread".to_string(),
                    artifact_id: artifact_id.clone(),
                    start_line: None,
                    limit: None,
                })
                .unwrap(),
            ),
        })
        .await;
    assert!(
        denied
            .error
            .unwrap()
            .message
            .contains("does not belong to thread")
    );

    let deleted: ArtifactDeleteResult = request(
        &client,
        "artifact/delete",
        Some(
            serde_json::to_value(ArtifactDeleteParams {
                thread_id: "app-server".to_string(),
                artifact_id,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(deleted.deleted);

    request::<SettingsSetFileBackedDynamicContextResult>(
        &client,
        "settings/set_file_backed_dynamic_context",
        Some(
            serde_json::to_value(SettingsSetFileBackedDynamicContextParams { enabled: false })
                .unwrap(),
        ),
    )
    .await;
    let disabled_command: serde_json::Value = request(
        &client,
        "command/exec",
        Some(serde_json::json!({
            "command": ["sh", "-c", "printf 'alpha\nneedle\nomega\n'"],
            "cwd": dir.display().to_string(),
            "processId": "process-artifact-disabled",
            "outputBytesCap": 8,
            "timeoutMs": 5000
        })),
    )
    .await;
    assert!(disabled_command.get("stdoutArtifact").is_none());
    assert_eq!(disabled_command["stdout"], "alpha\nne");

    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn team_methods_start_list_read_message_and_cleanup() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let started: TeamStartResult = request(
        &client,
        "team/start",
        Some(
            serde_json::to_value(TeamStartParams {
                lead_thread_id: None,
                display_mode: Some(roder_api::teams::AgentTeamDisplayMode::InProcess),
                members: vec![TeamStartMemberParams {
                    name: "Builder".to_string(),
                    model_provider: None,
                    model: None,
                }],
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(started.team.members.len(), 2);
    assert_eq!(started.team.members[1].name, "Builder");

    let team_started = wait_for_notification(&mut notifications, "team/started", None).await;
    assert_eq!(team_started.params["team"]["id"], started.team.id);

    let listed: TeamListResult = request(
        &client,
        "team/list",
        Some(serde_json::to_value(TeamListParams { limit: None }).unwrap()),
    )
    .await;
    assert!(listed.data.iter().any(|team| team.id == started.team.id));

    let added: TeamMemberStartResult = request(
        &client,
        "team/member/start",
        Some(
            serde_json::to_value(TeamMemberStartParams {
                team_id: started.team.id.clone(),
                name: "Reviewer".to_string(),
                model_provider: None,
                model: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(added.member.name, "Reviewer");

    let member_id = started.team.members[1].id.clone();
    let turn: TeamMemberMessageResult = request(
        &client,
        "team/member/message",
        Some(
            serde_json::to_value(TeamMemberMessageParams {
                team_id: started.team.id.clone(),
                member_id: member_id.clone(),
                text: "build it".to_string(),
                expected_turn_id: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!turn.turn_id.is_empty());
    let delta = wait_for_notification(&mut notifications, "team/member/messageDelta", None).await;
    assert_eq!(delta.params["teamId"], started.team.id);
    assert_eq!(delta.params["memberId"], member_id);
    assert_eq!(delta.params["turnId"], turn.turn_id);
    assert!(
        delta.params["delta"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );

    let read: TeamReadResult = request(
        &client,
        "team/read",
        Some(
            serde_json::to_value(TeamReadParams {
                team_id: started.team.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(read.team.is_some());
    assert_eq!(read.messages.len(), 1);
    assert_eq!(read.messages[0].to_member_id, member_id);
    assert_eq!(read.messages[0].text, "build it");

    let cleanup: TeamCleanupResult = request(
        &client,
        "team/cleanup",
        Some(
            serde_json::to_value(TeamCleanupParams {
                team_id: started.team.id.clone(),
                force: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(cleanup.cleaned);
}

#[tokio::test]
async fn team_member_interrupt_targets_only_requested_member() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let started: TeamStartResult = request(
        &client,
        "team/start",
        Some(
            serde_json::to_value(TeamStartParams {
                lead_thread_id: None,
                display_mode: Some(roder_api::teams::AgentTeamDisplayMode::InProcess),
                members: vec![
                    TeamStartMemberParams {
                        name: "A".to_string(),
                        model_provider: None,
                        model: None,
                    },
                    TeamStartMemberParams {
                        name: "B".to_string(),
                        model_provider: None,
                        model: None,
                    },
                ],
            })
            .unwrap(),
        ),
    )
    .await;
    let member_a = started.team.members[1].id.clone();
    let member_b = started.team.members[2].id.clone();

    let turn_a: TeamMemberMessageResult = request(
        &client,
        "team/member/message",
        Some(
            serde_json::to_value(TeamMemberMessageParams {
                team_id: started.team.id.clone(),
                member_id: member_a.clone(),
                text: "wait a".to_string(),
                expected_turn_id: None,
            })
            .unwrap(),
        ),
    )
    .await;
    let turn_b: TeamMemberMessageResult = request(
        &client,
        "team/member/message",
        Some(
            serde_json::to_value(TeamMemberMessageParams {
                team_id: started.team.id.clone(),
                member_id: member_b.clone(),
                text: "wait b".to_string(),
                expected_turn_id: None,
            })
            .unwrap(),
        ),
    )
    .await;

    let interrupted: TeamMemberInterruptResult = request(
        &client,
        "team/member/interrupt",
        Some(
            serde_json::to_value(TeamMemberInterruptParams {
                team_id: started.team.id.clone(),
                member_id: member_a.clone(),
                turn_id: Some(turn_a.turn_id.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(interrupted.interrupted);
    assert_eq!(
        interrupted.turn_id.as_deref(),
        Some(turn_a.turn_id.as_str())
    );

    let read: TeamReadResult = request(
        &client,
        "team/read",
        Some(
            serde_json::to_value(TeamReadParams {
                team_id: started.team.id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    let team = read.team.unwrap();
    let member_a_status = team
        .members
        .iter()
        .find(|member| member.id == member_a)
        .unwrap()
        .status;
    let member_b_state = team
        .members
        .iter()
        .find(|member| member.id == member_b)
        .unwrap();
    assert_eq!(
        member_a_status,
        roder_api::teams::TeamMemberStatus::Interrupted
    );
    assert_eq!(
        member_b_state.current_turn_id.as_deref(),
        Some(turn_b.turn_id.as_str())
    );
    assert_eq!(
        member_b_state.status,
        roder_api::teams::TeamMemberStatus::Running
    );
}

#[tokio::test]
async fn team_split_pane_only_methods_return_precise_headless_error() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("team/pane/focus")),
            method: "team/pane/focus".to_string(),
            params: Some(serde_json::json!({ "teamId": "team", "memberId": "member" })),
        })
        .await;
    let err = res.error.unwrap();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("split-pane TUI backend"));
    assert_eq!(
        err.data.unwrap()["supportedAlternative"],
        "team/member/focus"
    );
}

#[tokio::test]
async fn tools_list_discovers_configured_web_search_without_secret_material() {
    let secret = "secret-tavily-key";
    let registry = build_default_registry(DefaultRegistryConfig {
        web_search: Some(DefaultWebSearchConfig {
            enabled: true,
            tavily: DefaultWebSearchProviderConfig {
                enabled: true,
                api_key: Some(secret.to_string()),
                ..DefaultWebSearchProviderConfig::default()
            },
            ..DefaultWebSearchConfig::default()
        }),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let tools: ToolsListResult = request(&client, "tools/list", None).await;
    assert!(
        tools.tools.iter().any(|tool| tool.name == "web_search"),
        "tools/list should expose web_search: {:?}",
        tools.tools
    );

    let extensions: ExtensionsListResult = request(&client, "extensions/list", None).await;
    let protocol_text = serde_json::to_string(&(tools, extensions)).unwrap();
    assert!(!protocol_text.contains(secret));
    assert!(!protocol_text.contains("Authorization"));
    assert!(!protocol_text.contains("x-api-key"));
    assert!(!protocol_text.contains("api_key"));
}

#[tokio::test]
async fn discovery_methods_refresh_search_read_promote_list_and_clear() {
    let _guard = DISCOVERY_TEST_LOCK.lock().await;
    let root = std::env::temp_dir().join(format!(
        "roder-discovery-e2e-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let workspace = root.join("workspace");
    let catalog_dir = root.join("catalog");
    let state_dir = root.join("state");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join(".mcp.json"),
        r#"{"mcpServers":{"mcp-local":{"command":"node","env":{"API_KEY":"secret"}}}}"#,
    )
    .unwrap();
    unsafe {
        std::env::set_var("RODER_DISCOVERY_CATALOG_DIR", &catalog_dir);
        std::env::set_var("RODER_DISCOVERY_STATE_DIR", &state_dir);
    }

    let registry = build_default_registry(DefaultRegistryConfig {
        workspace: Some(workspace.clone()),
        ..Default::default()
    })
    .unwrap();
    let runtime = Arc::new(
        Runtime::new(
            registry,
            RuntimeConfig {
                workspace: Some(workspace.display().to_string()),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let refresh: DiscoveryRefreshResult = request(&client, "discovery/refresh", None).await;
    assert!(refresh.catalog_root.ends_with("catalog"));
    assert!(
        refresh
            .written_files
            .iter()
            .any(|path| path.ends_with("index.json"))
    );
    assert!(
        refresh
            .catalog
            .groups
            .iter()
            .any(|group| group.id == "tools:builtin-coding-tools")
    );

    let groups: DiscoveryGroupsResult = request(
        &client,
        "discovery/groups",
        Some(
            serde_json::to_value(DiscoveryGroupsParams {
                refresh: Some(false),
                limit: Some(50),
            })
            .unwrap(),
        ),
    )
    .await;
    let mcp_item = groups
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|item| item.name == "mcp-local")
        .expect("mcp discovery item");
    assert_eq!(
        mcp_item.source.auth_state,
        roder_api::discovery::DiscoveryAuthState::Required
    );
    assert!(mcp_item.redaction.redacted);
    assert!(
        mcp_item
            .redaction
            .fields
            .iter()
            .any(|field| field == "$.env")
    );

    let search: DiscoverySearchResult = request(
        &client,
        "discovery/search",
        Some(
            serde_json::to_value(DiscoverySearchParams {
                query: "grep".to_string(),
                refresh: Some(false),
                limit: Some(20),
            })
            .unwrap(),
        ),
    )
    .await;
    let grep_item = search
        .items
        .iter()
        .find(|item| item.id == "tool:builtin-coding-tools/grep")
        .expect("grep discovery item");

    let read: DiscoveryReadResult = request(
        &client,
        "discovery/read",
        Some(
            serde_json::to_value(DiscoveryReadParams {
                item_id: grep_item.id.clone(),
                refresh: Some(false),
                start_line: Some(1),
                limit: Some(20),
                promote: Some(true),
                thread_id: Some("thread-discovery".to_string()),
                turn_id: Some("turn-discovery".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(read.promoted);
    assert!(read.page.text.contains("\"query\""));
    assert!(read.page.total_lines <= 20 || read.page.truncated);

    let promote: DiscoveryPromoteResult = request(
        &client,
        "discovery/promote",
        Some(
            serde_json::to_value(DiscoveryPromoteParams {
                item_id: grep_item.id.clone(),
                thread_id: "thread-discovery".to_string(),
                turn_id: Some("turn-discovery-2".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        promote.record.promotion,
        roder_api::discovery::DiscoveryPromotionState::Reused
    );
    assert_eq!(
        promote.record.cache_status,
        roder_api::discovery::DiscoveryCacheStatus::Hit
    );

    let promoted: DiscoveryPromotedListResult = request(
        &client,
        "discovery/promoted/list",
        Some(
            serde_json::to_value(DiscoveryPromotedListParams {
                thread_id: Some("thread-discovery".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(promoted.records.len(), 1);

    let cleared: DiscoveryPromotedClearResult = request(
        &client,
        "discovery/promoted/clear",
        Some(
            serde_json::to_value(DiscoveryPromotedClearParams {
                thread_id: Some("thread-discovery".to_string()),
                item_id: Some(grep_item.id.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(cleared.cleared, 1);

    unsafe {
        std::env::remove_var("RODER_DISCOVERY_CATALOG_DIR");
        std::env::remove_var("RODER_DISCOVERY_STATE_DIR");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn retrieval_methods_read_recommendations_metrics_and_promotions() {
    let store: Arc<dyn ThreadStoreFactory> = Arc::new(RecordingThreadStoreFactory::default());
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(store);
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime.clone())));

    let thread_start = start_thread(&client).await;
    let thread_id = thread_start.thread.id.clone();
    let turn_id = "turn-retrieval".to_string();
    let route_id = "route-retrieval".to_string();
    let timestamp = OffsetDateTime::UNIX_EPOCH;
    let plan = RetrievalRoutePlan {
        route_id: route_id.clone(),
        thread_id: thread_id.clone(),
        turn_id: turn_id.clone(),
        intent: RetrievalIntent::InspectTool,
        recommended: vec![RetrievalRecommendation {
            mode: RetrievalMode::Discovery,
            tool: "discovery.search".to_string(),
            query: "grep".to_string(),
            reason: "tool lookup should start from discovery".to_string(),
            confidence: RetrievalConfidence::High,
            item_id: Some("tool:builtin-coding-tools/grep".to_string()),
        }],
        avoid: Vec::new(),
        timestamp,
    };
    runtime
        .emit(roder_api::events::RoderEvent::RetrievalRoutePlanned(
            RetrievalRoutePlanned { plan },
        ))
        .await;
    runtime
        .emit(roder_api::events::RoderEvent::RetrievalRouteAccepted(
            RetrievalRouteAccepted {
                route_id: route_id.clone(),
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                mode: RetrievalMode::Discovery,
                tool: "discovery.search".to_string(),
                query: "grep".to_string(),
                timestamp,
            },
        ))
        .await;
    runtime
        .emit(roder_api::events::RoderEvent::RetrievalRouteIgnored(
            RetrievalRouteIgnored {
                route_id: route_id.clone(),
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                chosen_tool: "web_search".to_string(),
                recommended_modes: vec![RetrievalMode::Discovery],
                reason: "model picked web for local tool lookup".to_string(),
                timestamp,
            },
        ))
        .await;
    runtime
        .emit(roder_api::events::RoderEvent::RetrievalResultUsed(
            RetrievalResultUsed {
                outcome: RetrievalMeasuredOutcome {
                    route_id: route_id.clone(),
                    mode: RetrievalMode::Discovery,
                    tool: "discovery.search".to_string(),
                    outcome: RetrievalOutcomeKind::Useful,
                    first_useful_path: Some(RetrievalMode::Discovery),
                    discovery_before_tool_use: true,
                    promotion_before_tool_use: false,
                    wrong_tool_family_attempts: 1,
                    result_count: 3,
                    latency_ms: 7,
                    bytes_returned: 512,
                    estimated_tokens_returned: 128,
                },
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                timestamp,
            },
        ))
        .await;
    runtime
        .emit(roder_api::events::RoderEvent::RetrievalPromotionSkipped(
            RetrievalPromotionSkipped {
                route_id,
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                item_id: "tool:builtin-coding-tools/grep".to_string(),
                reason: "already warm-cached".to_string(),
                timestamp,
            },
        ))
        .await;

    let params = Some(
        serde_json::to_value(RetrievalTurnParams {
            thread_id: thread_id.clone(),
            turn_id: turn_id.clone(),
            limit: Some(10),
        })
        .unwrap(),
    );
    let recommendations: RetrievalRecommendationsResult =
        request(&client, "retrieval/recommendations", params.clone()).await;
    assert_eq!(recommendations.plans.len(), 1);
    assert!(
        recommendations.summary.notes[0].contains("discovery.search"),
        "{:?}",
        recommendations.summary
    );

    let metrics: RetrievalMetricsResult =
        request(&client, "retrieval/metrics", params.clone()).await;
    assert_eq!(metrics.accepted_count, 1);
    assert_eq!(metrics.ignored_count, 1);
    assert_eq!(metrics.outcomes.len(), 1);
    assert_eq!(metrics.outcomes[0].wrong_tool_family_attempts, 1);

    let promoted: RetrievalPromotedResult = request(&client, "retrieval/promoted", params).await;
    assert_eq!(promoted.states.len(), 1);
    assert_eq!(promoted.states[0].state, "skipped");
    assert_eq!(
        promoted.states[0].reason.as_deref(),
        Some("already warm-cached")
    );
}

#[tokio::test]
async fn tools_list_exposes_default_coding_tools() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let tools: ToolsListResult = request(&client, "tools/list", None).await;
    let grep = tools
        .tools
        .iter()
        .find(|tool| tool.name == "grep")
        .expect("tools/list should expose grep");
    let grep_properties = grep.parameters["properties"]
        .as_object()
        .expect("grep parameters should expose object properties");
    assert!(grep_properties.contains_key("regex"));
    assert!(grep_properties.contains_key("case_sensitive"));
    assert!(grep_properties.contains_key("word_boundary"));
    assert_eq!(
        grep_properties["mode"]["enum"],
        serde_json::json!(["auto", "indexed", "scan"])
    );
    let names = tools
        .tools
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    for expected in [
        "read_file",
        "list_files",
        "grep",
        "glob",
        "shell",
        "exec_command",
        "write_stdin",
        "update_plan",
        "get_goal",
        "create_goal",
        "update_goal",
        "request_user_input",
        "write_file",
        "edit",
        "multi_edit",
        "spawn_agent",
        "send_message",
        "followup_task",
        "wait_agent",
        "list_agents",
        "close_agent",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "tools/list should expose {expected}: {names:?}"
        );
    }
}

#[tokio::test]
async fn extensions_list_exposes_capability_statuses() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let extensions: ExtensionsListResult = request(&client, "extensions/list", None).await;
    let statuses = extensions
        .capability_statuses
        .get("roder-ext-builtin-coding-tools")
        .expect("missing builtin coding tool capabilities");

    assert!(statuses.iter().any(|status| {
        status.id == "fs.read.workspace" && status.decision == CapabilityDecision::Requested
    }));
    assert!(statuses.iter().any(|status| {
        status.id == "process.spawn.shell" && status.decision == CapabilityDecision::Requested
    }));
}

#[tokio::test]
async fn commands_list_expand_and_skills_are_deterministic() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let first: CommandsListResult = request(&client, "commands/list", None).await;
    let second: CommandsListResult = request(&client, "commands/list", None).await;
    assert_eq!(
        first
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        second
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        first
            .commands
            .iter()
            .any(|command| command.name == "compact")
    );
    let marketplace = first
        .commands
        .iter()
        .find(|command| command.name == "marketplace")
        .expect("missing marketplace slash command");
    assert_eq!(
        marketplace.argument_hint.as_deref(),
        Some("list|install-default|add|remove|refresh|search|show [args]")
    );
    let plugin = first
        .commands
        .iter()
        .find(|command| command.name == "plugin")
        .expect("missing plugin slash command");
    assert_eq!(
        plugin.argument_hint.as_deref(),
        Some("preview|install|install-all|list|disable|uninstall [args]")
    );
    assert!(
        first
            .commands
            .iter()
            .any(|command| command.name == "commit")
    );

    let expanded: CommandsExpandResult = request(
        &client,
        "commands/expand",
        Some(
            serde_json::to_value(CommandsExpandParams {
                name: "compact".to_string(),
                arguments: "keep failures".to_string(),
                workspace: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(expanded.command.name, "compact");
    assert!(expanded.message.contains("Compact the current thread"));
    assert!(expanded.context_blocks.is_empty());
    let encoded = serde_json::to_string(&expanded).unwrap();
    assert!(!encoded.contains("secret-include-content"));

    let commit: CommandsExpandResult = request(
        &client,
        "commands/expand",
        Some(
            serde_json::to_value(CommandsExpandParams {
                name: "commit".to_string(),
                arguments: "src/lib.rs".to_string(),
                workspace: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(commit.command.name, "commit");
    assert!(commit.message.contains("bound commit skill"));
    assert!(commit.context_blocks.iter().any(|block| {
        block.text.starts_with("<skill name=\"commit\"") && block.text.contains("git status")
    }));
}

#[tokio::test]
async fn skills_manager_can_disable_commit_and_update_exposure() {
    let _guard = SKILLS_TEST_LOCK.lock().await;
    let root =
        std::env::temp_dir().join(format!("roder-skills-manager-e2e-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let previous_home = std::env::var_os("HOME");
    let previous_marketplaces_path = std::env::var_os("RODER_MARKETPLACES_PATH");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("RODER_MARKETPLACES_PATH", root.join("marketplaces.json"));
    }

    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .set_skills(roder_config::build_skills_registry(&workspace, None))
        .await;
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let listed: SkillsListResult = request(&client, "skills/list", None).await;
    let commit = listed
        .skills
        .iter()
        .find(|skill| skill.name == "commit")
        .expect("missing built-in commit skill");
    assert_eq!(commit.exposure, SkillExposure::DirectOnly);

    let updated: SkillsUpdateResult = request(
        &client,
        "skills/setExposure",
        Some(
            serde_json::to_value(SkillsSetExposureParams {
                selector: SkillSelector::Name {
                    name: "commit".to_string(),
                },
                exposure: SkillExposure::Global,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        updated
            .skills
            .iter()
            .find(|skill| skill.name == "commit")
            .unwrap()
            .exposure,
        SkillExposure::Global
    );

    let updated: SkillsUpdateResult = request(
        &client,
        "skills/setEnabled",
        Some(
            serde_json::to_value(SkillsSetEnabledParams {
                selector: SkillSelector::Name {
                    name: "commit".to_string(),
                },
                enabled: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        updated
            .skills
            .iter()
            .find(|skill| skill.name == "commit")
            .unwrap()
            .activation,
        SkillActivationState::Disabled
    );

    let error = request_error(
        &client,
        "commands/expand",
        Some(
            serde_json::to_value(CommandsExpandParams {
                name: "commit".to_string(),
                arguments: String::new(),
                workspace: Some(workspace.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(error.message.contains("disabled"));
    unsafe {
        if let Some(value) = previous_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
        if let Some(value) = previous_marketplaces_path {
            std::env::set_var("RODER_MARKETPLACES_PATH", value);
        } else {
            std::env::remove_var("RODER_MARKETPLACES_PATH");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn automations_create_run_now_status_and_runs() {
    let root = std::env::temp_dir().join(format!("roder-automations-e2e-{}", uuid::Uuid::new_v4()));
    let project = root.join("project");
    let thread_root = root.join("threads");
    let store_path = root.join("automations.sqlite3");
    std::fs::create_dir_all(&project).unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root,
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(project.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig {
            automations: roder_automations::AutomationSupervisorConfig {
                enabled: false,
                store_path: store_path.clone(),
                ..roder_automations::AutomationSupervisorConfig::default()
            },
        },
    ));
    let client = LocalAppClient::new(server);

    let status: AutomationsStatusResult = request(&client, "automations/status", None).await;
    assert!(!status.scheduler_enabled);
    assert!(status.read_api_enabled);

    let created: AutomationsCreateResult = request(
        &client,
        "automations/create",
        Some(
            serde_json::to_value(AutomationsCreateParams {
                name: "Hourly status".to_string(),
                project: AutomationProject {
                    cwd: project.display().to_string(),
                    display_name: Some("project".to_string()),
                },
                schedule: AutomationSchedule::Interval { seconds: 60 },
                prompt: "say hello".to_string(),
                enabled: true,
                model_provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                policy_mode: None,
                catch_up: CatchUpPolicy::RunLatestOnly,
                concurrency: AutomationConcurrencyPolicy::Forbid,
            })
            .unwrap(),
        ),
    )
    .await;

    let listed: AutomationsListResult = request(&client, "automations/list", None).await;
    assert_eq!(listed.automations.len(), 1);
    assert_eq!(listed.automations[0].id, created.automation.id);

    let run_now: AutomationsRunNowResult = request(
        &client,
        "automations/runNow",
        Some(
            serde_json::to_value(AutomationsRunNowParams {
                automation_id: created.automation.id.clone(),
                prompt_override: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(run_now.run.state, AutomationRunState::Queued);

    let completed = wait_for_automation_run(
        &client,
        &created.automation.id,
        &run_now.run.run_id,
        AutomationRunState::Completed,
    )
    .await;
    assert!(completed.thread_id.is_some());
    assert!(completed.turn_id.is_some());
    assert!(completed.task_id.is_some());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn automations_create_due_tick_run_and_persisted_thread_read() {
    let root = std::env::temp_dir().join(format!(
        "roder-automations-scheduled-e2e-{}",
        uuid::Uuid::new_v4()
    ));
    let project = root.join("project");
    let thread_root = root.join("threads");
    let store_path = root.join("automations.sqlite3");
    std::fs::create_dir_all(&project).unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root,
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(project.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig {
            automations: roder_automations::AutomationSupervisorConfig {
                enabled: true,
                store_path: store_path.clone(),
                tick_seconds: 1,
                lease_seconds: 30,
                max_due_per_tick: 5,
                run_missed_on_startup: false,
                ..roder_automations::AutomationSupervisorConfig::default()
            },
        },
    ));
    let client = LocalAppClient::new(server);

    let created: AutomationsCreateResult = request(
        &client,
        "automations/create",
        Some(
            serde_json::to_value(AutomationsCreateParams {
                name: "Scheduled status".to_string(),
                project: AutomationProject {
                    cwd: project.display().to_string(),
                    display_name: Some("project".to_string()),
                },
                schedule: AutomationSchedule::Interval { seconds: 1 },
                prompt: "say hello from scheduled automation".to_string(),
                enabled: true,
                model_provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                policy_mode: None,
                catch_up: CatchUpPolicy::RunLatestOnly,
                concurrency: AutomationConcurrencyPolicy::Forbid,
            })
            .unwrap(),
        ),
    )
    .await;

    let completed = wait_for_automation_state(
        &client,
        &created.automation.id,
        AutomationRunState::Completed,
        180,
    )
    .await;
    let thread_id = completed
        .thread_id
        .clone()
        .expect("completed automation records a thread id");
    assert!(completed.turn_id.is_some());
    assert!(completed.task_id.is_some());

    let read: ThreadReadResult = request(
        &client,
        "thread/read",
        Some(
            serde_json::to_value(ThreadReadParams {
                thread_id: thread_id.clone(),
                include_turns: true,
            })
            .unwrap(),
        ),
    )
    .await;
    let thread = read.thread.expect("automation thread is persisted");
    assert_eq!(thread.id, thread_id);
    assert!(thread.turns.unwrap_or_default().iter().any(|turn| {
        turn.id
            == completed
                .turn_id
                .clone()
                .expect("completed automation records a turn id")
    }));

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[ignore]
async fn automations_live_wall_clock_tick_smoke() {
    if std::env::var("RODER_LIVE_AUTOMATIONS").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_AUTOMATIONS=1 to run the live automation smoke");
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "roder-automations-live-e2e-{}",
        uuid::Uuid::new_v4()
    ));
    let project = root.join("project");
    let store_path = root.join("automations.sqlite3");
    std::fs::create_dir_all(&project).unwrap();
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig {
            automations: roder_automations::AutomationSupervisorConfig {
                enabled: true,
                store_path: store_path.clone(),
                tick_seconds: 1,
                lease_seconds: 30,
                max_due_per_tick: 1,
                run_missed_on_startup: false,
                ..roder_automations::AutomationSupervisorConfig::default()
            },
        },
    ));
    let client = LocalAppClient::new(server);

    let created: AutomationsCreateResult = request(
        &client,
        "automations/create",
        Some(
            serde_json::to_value(AutomationsCreateParams {
                name: "Live scheduled status".to_string(),
                project: AutomationProject {
                    cwd: project.display().to_string(),
                    display_name: Some("project".to_string()),
                },
                schedule: AutomationSchedule::Interval { seconds: 1 },
                prompt: "say hello from live automation smoke".to_string(),
                enabled: true,
                model_provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                policy_mode: None,
                catch_up: CatchUpPolicy::RunLatestOnly,
                concurrency: AutomationConcurrencyPolicy::Forbid,
            })
            .unwrap(),
        ),
    )
    .await;

    let completed = wait_for_automation_state(
        &client,
        &created.automation.id,
        AutomationRunState::Completed,
        240,
    )
    .await;
    assert!(completed.thread_id.is_some());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn discovery_catalog_includes_runtime_skills() {
    let _guard = SKILLS_TEST_LOCK.lock().await;
    let root = std::env::temp_dir().join(format!(
        "roder-skills-discovery-e2e-{}",
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let previous_catalog = std::env::var_os("RODER_DISCOVERY_CATALOG_DIR");
    let previous_state = std::env::var_os("RODER_DISCOVERY_STATE_DIR");
    unsafe {
        std::env::set_var("RODER_DISCOVERY_CATALOG_DIR", root.join("catalog"));
        std::env::set_var("RODER_DISCOVERY_STATE_DIR", root.join("state"));
    }

    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .set_skills(roder_config::build_skills_registry(&workspace, None))
        .await;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    let groups: DiscoveryGroupsResult = request(
        &client,
        "discovery/groups",
        Some(serde_json::json!({ "refresh": true })),
    )
    .await;
    let skills_group = groups
        .groups
        .iter()
        .find(|group| group.id == "skills:registry")
        .expect("skills discovery group");
    let commit = skills_group
        .items
        .iter()
        .find(|item| item.name == "commit")
        .expect("commit discovery item");
    assert_eq!(commit.source.kind, DiscoverySourceKind::Skills);
    assert!(commit.tags.contains(&"built-in".to_string()));
    assert!(commit.tags.contains(&"direct-only".to_string()));

    unsafe {
        if let Some(value) = previous_catalog {
            std::env::set_var("RODER_DISCOVERY_CATALOG_DIR", value);
        } else {
            std::env::remove_var("RODER_DISCOVERY_CATALOG_DIR");
        }
        if let Some(value) = previous_state {
            std::env::set_var("RODER_DISCOVERY_STATE_DIR", value);
        } else {
            std::env::remove_var("RODER_DISCOVERY_STATE_DIR");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn commands_run_expands_and_starts_turn() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let thread_start = start_thread(&client).await;
    let result: CommandsRunResult = request(
        &client,
        "commands/run",
        Some(
            serde_json::to_value(CommandsRunParams {
                thread_id: thread_start.thread.id.clone(),
                name: "init".to_string(),
                arguments: String::new(),
                workspace: None,
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!result.turn_id.is_empty());
    assert_eq!(result.expanded.command.name, "init");
    wait_for_event(&mut events, &thread_start.thread.id, "turn.completed").await;
}

#[tokio::test]
async fn tasks_submit_list_get_and_events_observe_process_task() {
    let workspace =
        std::env::temp_dir().join(format!("roder-app-server-task-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let registry = build_default_registry(DefaultRegistryConfig {
        workspace: Some(workspace.clone()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let submitted: TasksSubmitResult = request(
        &client,
        "tasks/submit",
        Some(
            serde_json::to_value(TasksSubmitParams {
                executor_id: "process".to_string(),
                input: serde_json::json!({
                    "command": "sh",
                    "args": ["-c", "printf task-ok"],
                    "cwd": ".",
                }),
                thread_id: None,
                turn_id: None,
                workspace: Some(workspace.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(submitted.task.executor_id, "process");

    let listed: TasksListResult = request(&client, "tasks/list", None).await;
    assert!(
        listed
            .tasks
            .iter()
            .any(|task| task.task_id == submitted.task.task_id),
        "submitted task missing from list: {:?}",
        listed.tasks
    );

    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.kind == "task.completed"
            && matches!(&envelope.event, roder_api::events::RoderEvent::TaskCompleted(event) if event.task_id == submitted.task.task_id)
        {
            break;
        }
    }

    let observed: TasksGetResult = request(
        &client,
        "tasks/get",
        Some(
            serde_json::to_value(TasksGetParams {
                task_id: submitted.task.task_id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(observed.task.state, TaskState::Completed);
    assert_eq!(
        observed
            .logs
            .iter()
            .map(|entry| entry.chunk.as_str())
            .collect::<String>(),
        "task-ok"
    );
}

#[tokio::test]
async fn processes_list_get_stop_and_subscribe_for_process_task() {
    let workspace =
        std::env::temp_dir().join(format!("roder-app-server-process-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let registry = build_default_registry(DefaultRegistryConfig {
        workspace: Some(workspace.clone()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let subscribed: roder_protocol::ProcessesSubscribeResult =
        request(&client, "processes/subscribe", None).await;
    assert!(subscribed.subscribed);
    assert!(
        subscribed
            .event_kinds
            .iter()
            .any(|kind| kind == "process.started")
    );

    let submitted: TasksSubmitResult = request(
        &client,
        "tasks/submit",
        Some(
            serde_json::to_value(TasksSubmitParams {
                executor_id: "process".to_string(),
                input: serde_json::json!({
                    "command": "sh",
                    "args": ["-c", "printf 'process-ready\n'; sleep 5"],
                    "cwd": ".",
                }),
                thread_id: Some("thread-process".to_string()),
                turn_id: Some("turn-process".to_string()),
                workspace: Some(workspace.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;

    let process = wait_for_process_by_task(&client, &submitted.task.task_id).await;
    assert_eq!(
        process.task_id.as_deref(),
        Some(submitted.task.task_id.as_str())
    );
    assert_eq!(process.thread_id.as_deref(), Some("thread-process"));
    assert!(process.stoppable);

    let detail: ProcessesGetResult = request(
        &client,
        "processes/get",
        Some(
            serde_json::to_value(ProcessesGetParams {
                process_id: process.process_id.clone(),
                output_bytes: Some(4096),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(
        detail.process.as_ref().unwrap().process_id,
        process.process_id
    );
    let mut saw_output = false;
    for _ in 0..50 {
        let detail: ProcessesGetResult = request(
            &client,
            "processes/get",
            Some(
                serde_json::to_value(ProcessesGetParams {
                    process_id: process.process_id.clone(),
                    output_bytes: Some(4096),
                })
                .unwrap(),
            ),
        )
        .await;
        if detail
            .output
            .iter()
            .any(|output| output.chunk.contains("process-ready"))
        {
            saw_output = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_output, "process output tail missing process-ready");

    let stopped: ProcessesStopResult = request(
        &client,
        "processes/stop",
        Some(
            serde_json::to_value(ProcessesStopParams {
                process_id: process.process_id.clone(),
                reason: Some("test stop".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(stopped.result.stopped);

    for _ in 0..50 {
        let detail: ProcessesGetResult = request(
            &client,
            "processes/get",
            Some(
                serde_json::to_value(ProcessesGetParams {
                    process_id: process.process_id.clone(),
                    output_bytes: Some(4096),
                })
                .unwrap(),
            ),
        )
        .await;
        if let Some(process) = detail.process.as_ref()
            && matches!(process.state, roder_api::processes::ProcessState::Stopped)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process did not stop");
}

#[tokio::test]
async fn processes_stop_all_stops_multiple_running_processes() {
    let workspace = std::env::temp_dir().join(format!(
        "roder-app-server-stop-all-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let registry = build_default_registry(DefaultRegistryConfig {
        workspace: Some(workspace.clone()),
        ..DefaultRegistryConfig::default()
    })
    .unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let first: TasksSubmitResult = request(
        &client,
        "tasks/submit",
        Some(
            serde_json::to_value(TasksSubmitParams {
                executor_id: "process".to_string(),
                input: serde_json::json!({
                    "command": "sh",
                    "args": ["-c", "sleep 5"],
                    "cwd": ".",
                }),
                thread_id: None,
                turn_id: None,
                workspace: Some(workspace.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    let second: TasksSubmitResult = request(
        &client,
        "tasks/submit",
        Some(
            serde_json::to_value(TasksSubmitParams {
                executor_id: "process".to_string(),
                input: serde_json::json!({
                    "command": "sh",
                    "args": ["-c", "sleep 5"],
                    "cwd": ".",
                }),
                thread_id: None,
                turn_id: None,
                workspace: Some(workspace.display().to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    let first_process = wait_for_process_by_task(&client, &first.task.task_id).await;
    let second_process = wait_for_process_by_task(&client, &second.task.task_id).await;

    let stopped: ProcessesStopAllResult = request(
        &client,
        "processes/stopAll",
        Some(
            serde_json::to_value(ProcessesStopAllParams {
                reason: Some("test stop all".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(
        stopped
            .results
            .iter()
            .any(|result| { result.process_id == first_process.process_id && result.stopped })
    );
    assert!(
        stopped
            .results
            .iter()
            .any(|result| { result.process_id == second_process.process_id && result.stopped })
    );
}

async fn wait_for_process_by_task(
    client: &LocalAppClient,
    task_id: &str,
) -> roder_api::processes::ProcessDescriptor {
    for _ in 0..50 {
        let listed: ProcessesListResult = request(
            client,
            "processes/list",
            Some(
                serde_json::to_value(ProcessesListParams {
                    include_completed: true,
                })
                .unwrap(),
            ),
        )
        .await;
        if let Some(process) = listed
            .processes
            .into_iter()
            .find(|process| process.task_id.as_deref() == Some(task_id))
        {
            return process;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process for task {task_id} not found");
}

#[tokio::test]
async fn tools_call_can_create_and_get_goal() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let thread_start = start_thread(&client).await;
    let created: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship slash goal",
                }),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!created.is_error, "create_goal failed: {created:?}");
    assert!(created.text.contains("Ship slash goal"));

    let replaced: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id.clone(),
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship replacement goal",
                    "token_budget": 200,
                }),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(
        !replaced.is_error,
        "create_goal replacement failed: {replaced:?}"
    );
    assert!(replaced.text.contains("Ship replacement goal"));
    assert_eq!(replaced.data["hasActiveGoal"], true);
    assert_eq!(replaced.data["goal"]["objective"], "Ship replacement goal");
    assert_eq!(replaced.data["goal"]["status"], "active");
    assert_eq!(replaced.data["goal"]["tokenBudget"], 200);
    assert_eq!(replaced.data["goal"]["tokensUsed"], 0);
    assert_eq!(replaced.data["goal"]["timeUsedSeconds"], 0);

    let current: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: thread_start.thread.id,
                tool_name: "get_goal".to_string(),
                arguments: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!current.is_error, "get_goal failed: {current:?}");
    assert!(current.text.contains("Ship replacement goal"));
}

#[tokio::test]
async fn request_user_input_tool_waits_for_app_server_resolution() {
    let engine = Arc::new(UserInputEngine {
        calls: Mutex::new(0),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(".").unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let mut notifications = client.subscribe_notifications();

    let thread_start = start_thread(&client).await;
    let _started = start_turn(&client, &thread_start.thread.id, "ask me").await;

    let requested_notification = wait_for_notification(
        &mut notifications,
        "thread/userInputRequested",
        Some(&thread_start.thread.id),
    )
    .await;
    assert_eq!(requested_notification.params["questions"][0]["id"], "mode");
    assert_eq!(
        requested_notification.params["requestId"]
            .as_str()
            .is_some(),
        true
    );

    let mut request_id = None;
    for _ in 0..20 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::UserInputRequested(event) = envelope.event {
            assert_eq!(event.questions[0]["id"], "mode");
            request_id = Some(event.request_id);
            break;
        }
    }
    let request_id = request_id.expect("missing user input request event");
    assert_eq!(
        requested_notification.params["requestId"].as_str(),
        Some(request_id.as_str())
    );

    let resolved: ThreadResolveUserInputResult = request(
        &client,
        "thread/resolve_user_input",
        Some(
            serde_json::to_value(ThreadResolveUserInputParams {
                request_id: request_id.clone(),
                answers: serde_json::json!({ "mode": "Safe" }),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(resolved.resolved);

    let resolved_notification = wait_for_notification(
        &mut notifications,
        "thread/userInputResolved",
        Some(&thread_start.thread.id),
    )
    .await;
    assert_eq!(resolved_notification.params["requestId"], request_id);
    assert_eq!(resolved_notification.params["answers"]["mode"], "Safe");

    let mut saw_resolved = false;
    let mut saw_turn_completed = false;
    for _ in 0..30 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            roder_api::events::RoderEvent::UserInputResolved(event) => {
                saw_resolved = event.request_id == request_id && event.answers["mode"] == "Safe";
            }
            roder_api::events::RoderEvent::TurnCompleted(_) => {
                saw_turn_completed = true;
                break;
            }
            _ => {}
        }
    }

    assert!(saw_resolved);
    assert!(saw_turn_completed);
}

#[tokio::test]
async fn web_search_setting_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert_eq!(settings.web_search.mode, HostedWebSearchMode::Cached);
    assert_eq!(settings.default_mode, PolicyMode::Default);
    assert!(settings.file_backed_dynamic_context);

    let changed: SettingsSetWebSearchResult = request(
        &client,
        "settings/set_web_search",
        Some(
            serde_json::to_value(SettingsSetWebSearchParams {
                mode: HostedWebSearchMode::Live,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(changed.web_search.mode, HostedWebSearchMode::Live);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert_eq!(settings.web_search.mode, HostedWebSearchMode::Live);
    assert_eq!(
        runtime.status().await.hosted_web_search.mode,
        HostedWebSearchMode::Live
    );
}

#[tokio::test]
async fn search_index_setting_can_be_set_and_observed() {
    let _guard = SEARCH_INDEX_TEST_LOCK.lock().await;
    roder_search::set_search_index_enabled(true);
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert!(settings.search_index.enabled);

    let changed: SettingsSetSearchIndexResult = request(
        &client,
        "settings/set_search_index",
        Some(serde_json::to_value(SettingsSetSearchIndexParams { enabled: false }).unwrap()),
    )
    .await;
    assert!(!changed.search_index.enabled);
    assert!(!roder_search::search_index_enabled());

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert!(!settings.search_index.enabled);
    roder_search::set_search_index_enabled(true);
}

#[tokio::test]
async fn shell_setting_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert!(!settings.shell.shell.trim().is_empty());
    assert!(settings.shell.options.contains(&"zsh".to_string()));
    assert!(settings.shell.options.contains(&"bash".to_string()));

    let changed: SettingsSetShellResult = request(
        &client,
        "settings/set_shell",
        Some(
            serde_json::to_value(SettingsSetShellParams {
                shell: "bash".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(changed.shell.shell, "bash");

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert_eq!(settings.shell.shell, "bash");
    assert_eq!(runtime.status().await.command_shell, "bash");
}

#[tokio::test]
async fn search_index_methods_manage_status_warmup_rebuild_and_clear() {
    let _guard = SEARCH_INDEX_TEST_LOCK.lock().await;
    roder_search::set_search_index_enabled(true);

    let root = std::env::temp_dir().join(format!(
        "roder-search-index-app-server-{}",
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let home = root.join("home");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("main.rs"),
        "fn main() { println!(\"needle\"); }\n",
    )
    .unwrap();
    unsafe {
        std::env::set_var("RODER_SEARCH_INDEX_HOME", &home);
    }

    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();
    let workspace_param = workspace.display().to_string();

    let status: SearchIndexStatusResult = request(
        &client,
        "search_index/status",
        Some(
            serde_json::to_value(SearchIndexStatusParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(status.status.state, SearchIndexStatusState::Missing);
    assert!(status.status.enabled);

    let warmup: SearchIndexWarmupResult = request(
        &client,
        "search_index/warmup",
        Some(
            serde_json::to_value(SearchIndexWarmupParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(warmup.status.state, SearchIndexStatusState::Ready);
    assert_eq!(warmup.status.document_count, Some(1));

    let building =
        wait_for_notification(&mut notifications, "search_index/statusChanged", None).await;
    assert_eq!(building.params["status"]["state"], "building");
    let ready = wait_for_notification(&mut notifications, "search_index/statusChanged", None).await;
    assert_eq!(ready.params["status"]["state"], "ready");
    assert_eq!(ready.params["status"]["documentCount"], 1);

    std::fs::write(
        workspace.join("main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    let stale: SearchIndexStatusResult = request(
        &client,
        "search_index/status",
        Some(
            serde_json::to_value(SearchIndexStatusParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(stale.status.state, SearchIndexStatusState::Stale);
    assert!(stale.status.stale);

    let rebuild: SearchIndexRebuildResult = request(
        &client,
        "search_index/rebuild",
        Some(
            serde_json::to_value(SearchIndexRebuildParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(rebuild.status.state, SearchIndexStatusState::Ready);

    let clear: SearchIndexClearResult = request(
        &client,
        "search_index/clear",
        Some(
            serde_json::to_value(SearchIndexClearParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(clear.status.state, SearchIndexStatusState::Cleared);
    let cleared =
        wait_for_notification(&mut notifications, "search_index/statusChanged", None).await;
    assert_eq!(cleared.params["status"]["state"], "building");
    let cleared =
        wait_for_notification(&mut notifications, "search_index/statusChanged", None).await;
    assert_eq!(cleared.params["status"]["state"], "ready");
    let cleared =
        wait_for_notification(&mut notifications, "search_index/statusChanged", None).await;
    assert_eq!(cleared.params["status"]["state"], "cleared");

    let missing: SearchIndexStatusResult = request(
        &client,
        "search_index/status",
        Some(
            serde_json::to_value(SearchIndexStatusParams {
                workspace: Some(workspace_param),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(missing.status.state, SearchIndexStatusState::Missing);

    roder_search::set_search_index_enabled(false);
    let disabled: SearchIndexStatusResult = request(
        &client,
        "search_index/status",
        Some(serde_json::to_value(SearchIndexStatusParams { workspace: None }).unwrap()),
    )
    .await;
    assert_eq!(disabled.status.state, SearchIndexStatusState::Disabled);
    roder_search::set_search_index_enabled(true);
    unsafe {
        std::env::remove_var("RODER_SEARCH_INDEX_HOME");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn code_index_methods_rebuild_search_read_chunks_and_list_proofs() {
    let root = std::env::temp_dir().join(format!(
        "roder-code-index-app-server-{}",
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let home = root.join("home");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(
        workspace.join("src/auth.rs"),
        "pub fn oauth_refresh_token() {\n    let token = \"refresh\";\n}\n",
    )
    .unwrap();
    unsafe {
        std::env::set_var("RODER_CODE_INDEX_HOME", &home);
    }

    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();
    let workspace_param = workspace.display().to_string();

    let status: CodeIndexStatusResult = request(
        &client,
        "index/status",
        Some(
            serde_json::to_value(CodeIndexStatusParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(status.status.status, CodeIndexStatus::Missing);

    let rebuild: CodeIndexRebuildResult = request(
        &client,
        "index/rebuild",
        Some(
            serde_json::to_value(CodeIndexRebuildParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(rebuild.status.status, CodeIndexStatus::Ready);
    assert_eq!(rebuild.status.stats.file_count, 1);
    let ready = wait_for_notification(&mut notifications, "index/statusChanged", None).await;
    assert_eq!(ready.params["status"]["status"], "ready");

    let search: CodeIndexSearchResultEnvelope = request(
        &client,
        "index/search",
        Some(
            serde_json::to_value(CodeIndexSearchParams {
                workspace: Some(workspace_param.clone()),
                query: "oauth refresh token".to_string(),
                limit: Some(5),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(search.status.status, CodeIndexStatus::Ready);
    assert_eq!(search.response.generation.status, CodeIndexStatus::Ready);
    assert!(!search.response.results.is_empty());
    assert!(search.response.results[0].proof_verified);
    let chunk_hash = search.response.results[0].chunk.chunk_hash.clone();

    let denied: JsonRpcError = request_error(
        &client,
        "index/readChunk",
        Some(
            serde_json::to_value(CodeIndexReadChunkParams {
                workspace: Some(workspace_param.clone()),
                chunk_hash: chunk_hash.clone(),
                offset: None,
                limit: None,
                include_source: false,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(denied.message.contains("includeSource=true"));

    let read: CodeIndexReadChunkResult = request(
        &client,
        "index/readChunk",
        Some(
            serde_json::to_value(CodeIndexReadChunkParams {
                workspace: Some(workspace_param.clone()),
                chunk_hash,
                offset: Some(0),
                limit: Some(32),
                include_source: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(read.page.total_bytes >= read.page.text.len());
    assert!(read.page.text.contains("oauth") || read.page.text.contains("token"));

    let proofs: CodeIndexProofsListResult = request(
        &client,
        "index/proofs/list",
        Some(
            serde_json::to_value(CodeIndexProofsListParams {
                workspace: Some(workspace_param.clone()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(!proofs.proofs.is_empty());
    assert_eq!(
        proofs.proofs[0].generation_id,
        rebuild.status.generation_id.unwrap()
    );

    std::fs::write(
        workspace.join("src/auth.rs"),
        "pub fn oauth_refresh_token_changed() {}\n",
    )
    .unwrap();
    let stale: CodeIndexStatusResult = request(
        &client,
        "index/status",
        Some(
            serde_json::to_value(CodeIndexStatusParams {
                workspace: Some(workspace_param),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(stale.status.status, CodeIndexStatus::Stale);
    assert!(stale.status.stale);

    unsafe {
        std::env::remove_var("RODER_CODE_INDEX_HOME");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn settings_file_backed_dynamic_context_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);

    let changed: SettingsSetFileBackedDynamicContextResult = request(
        &client,
        "settings/set_file_backed_dynamic_context",
        Some(
            serde_json::to_value(SettingsSetFileBackedDynamicContextParams { enabled: false })
                .unwrap(),
        ),
    )
    .await;
    assert!(!changed.enabled);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert!(!settings.file_backed_dynamic_context);
    assert!(!runtime.status().await.file_backed_dynamic_context);
}

#[tokio::test]
async fn settings_default_mode_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let changed: SettingsSetDefaultModeResult = request(
        &client,
        "settings/set_default_mode",
        Some(
            serde_json::to_value(SettingsSetDefaultModeParams {
                mode: PolicyMode::AcceptAll,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(changed.default_mode, PolicyMode::AcceptAll);

    let settings: SettingsGetResult = request(&client, "settings/get", None).await;
    assert_eq!(settings.default_mode, PolicyMode::AcceptAll);
    assert_eq!(runtime.status().await.policy_mode, PolicyMode::AcceptAll);

    let mut saw_mode_changed = false;
    for _ in 0..8 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::PolicyModeChanged(event) = envelope.event {
            saw_mode_changed = event.new_mode == PolicyMode::AcceptAll;
            break;
        }
    }
    assert!(saw_mode_changed);
}

#[tokio::test]
async fn thread_policy_mode_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let state: ThreadStateResult = request(&client, "thread/state", None).await;
    assert_eq!(state.mode, PolicyMode::Default);

    let changed: ThreadSetModeResult = request(
        &client,
        "thread/set_mode",
        Some(
            serde_json::to_value(ThreadSetModeParams {
                mode: PolicyMode::Plan,
                reason: Some("test".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(changed.mode, PolicyMode::Plan);

    let state: ThreadStateResult = request(&client, "thread/state", None).await;
    assert_eq!(state.mode, PolicyMode::Plan);

    let mut saw_mode_changed = false;
    for _ in 0..8 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::PolicyModeChanged(event) = envelope.event {
            saw_mode_changed = event.new_mode == PolicyMode::Plan;
            break;
        }
    }
    assert!(saw_mode_changed);
}

#[tokio::test]
async fn thread_exit_plan_resolves_pending_request() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .set_policy_mode(PolicyMode::Plan, Some("test setup".to_string()))
        .await
        .unwrap();
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let mut notifications = client.subscribe_notifications();

    runtime
        .record_pending_plan_exit(PendingPlanExit::new(
            "thread-plan".to_string(),
            "turn-plan".to_string(),
            "exit-plan-1".to_string(),
            PolicyMode::Default,
            Some("Implement approved edits".to_string()),
        ))
        .await;

    let requested_notification = wait_for_notification(
        &mut notifications,
        "thread/planExitRequested",
        Some("thread-plan"),
    )
    .await;
    assert_eq!(requested_notification.params["requestId"], "exit-plan-1");
    assert_eq!(requested_notification.params["targetMode"], "default");
    assert_eq!(
        requested_notification.params["planSummary"],
        "Implement approved edits"
    );

    let state: ThreadStateResult = request(&client, "thread/state", None).await;
    assert_eq!(
        state
            .pending_plan_exit
            .as_ref()
            .map(|pending| pending.request_id.as_str()),
        Some("exit-plan-1")
    );

    let resolved: ThreadExitPlanResult = request(
        &client,
        "thread/exit_plan",
        Some(
            serde_json::to_value(ThreadExitPlanParams {
                request_id: "exit-plan-1".to_string(),
                approved: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(resolved.resolved);
    assert_eq!(resolved.mode, PolicyMode::Default);

    let resolved_notification = wait_for_notification(
        &mut notifications,
        "thread/planExitResolved",
        Some("thread-plan"),
    )
    .await;
    assert_eq!(resolved_notification.params["requestId"], "exit-plan-1");
    assert_eq!(resolved_notification.params["approved"], true);
    assert_eq!(resolved_notification.params["resolvedMode"], "default");

    let state: ThreadStateResult = request(&client, "thread/state", None).await;
    assert_eq!(state.mode, PolicyMode::Default);
    assert!(state.pending_plan_exit.is_none());

    let mut saw_resolved = false;
    for _ in 0..8 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::PolicyExitPlanResolved(event) = envelope.event {
            saw_resolved = event.request_id == "exit-plan-1" && event.approved;
            break;
        }
    }
    assert!(saw_resolved);
}

#[tokio::test]
async fn thread_exit_plan_timeout_rejects_late_approval() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .set_policy_mode(PolicyMode::Plan, Some("test setup".to_string()))
        .await
        .unwrap();
    runtime
        .record_pending_plan_exit(PendingPlanExit {
            thread_id: "thread-plan".to_string(),
            turn_id: "turn-plan".to_string(),
            request_id: "exit-plan-expired".to_string(),
            target_mode: PolicyMode::Default,
            plan_summary: Some("Expired plan".to_string()),
            requested_at: OffsetDateTime::now_utc() - time::Duration::minutes(20),
            expires_at: Some(OffsetDateTime::now_utc() - time::Duration::seconds(1)),
        })
        .await;
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let resolved: ThreadExitPlanResult = request(
        &client,
        "thread/exit_plan",
        Some(
            serde_json::to_value(ThreadExitPlanParams {
                request_id: "exit-plan-expired".to_string(),
                approved: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(resolved.resolved);
    assert_eq!(resolved.mode, PolicyMode::Plan);

    let state: ThreadStateResult = request(&client, "thread/state", None).await;
    assert_eq!(state.mode, PolicyMode::Plan);
    assert!(state.pending_plan_exit.is_none());

    let mut saw_timeout_rejection = false;
    for _ in 0..8 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::PolicyExitPlanResolved(event) = envelope.event {
            saw_timeout_rejection = event.request_id == "exit-plan-expired" && !event.approved;
            break;
        }
    }
    assert!(saw_timeout_rejection);
}

#[tokio::test]
async fn task_tool_emits_subagent_events_before_tool_completion() {
    let runtime = subagent_runtime();
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let thread_start = start_thread(&client).await;
    let started = start_turn(&client, &thread_start.thread.id, "delegate this").await;

    let mut kinds = Vec::new();
    let mut child_parent_ids = Vec::new();
    for _ in 0..40 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        kinds.push(envelope.kind.clone());
        match envelope.event {
            roder_api::events::RoderEvent::SubagentStarted(started) => {
                child_parent_ids.push((started.parent_thread_id, started.parent_turn_id));
            }
            roder_api::events::RoderEvent::SubagentCompleted(completed) => {
                child_parent_ids.push((completed.parent_thread_id, completed.parent_turn_id));
            }
            roder_api::events::RoderEvent::SubagentTraceCreated(created) => {
                child_parent_ids.push((
                    created.summary.parent.thread_id,
                    created.summary.parent.turn_id,
                ));
            }
            roder_api::events::RoderEvent::SubagentTraceCompleted(completed) => {
                child_parent_ids.push((
                    completed.summary.parent.thread_id,
                    completed.summary.parent.turn_id,
                ));
            }
            _ => {}
        }
        if envelope.kind == "turn.completed" {
            break;
        }
    }

    let trace_created = position(&kinds, "turn/subagentTraceCreated");
    let trace_completed = position(&kinds, "turn/subagentTraceCompleted");
    let subagent_started = position(&kinds, "subagent.started");
    let subagent_completed = position(&kinds, "subagent.completed");
    let tool_completed = position(&kinds, "tool.call_completed");
    assert!(trace_created < trace_completed, "{kinds:?}");
    assert!(trace_completed < tool_completed, "{kinds:?}");
    assert!(subagent_started < subagent_completed, "{kinds:?}");
    assert!(subagent_completed < tool_completed, "{kinds:?}");
    assert!(
        position(&kinds, "turn.completed") > tool_completed,
        "{kinds:?}"
    );
    assert!(
        child_parent_ids
            .iter()
            .all(|(thread_id, turn_id)| thread_id == &thread_start.thread.id
                && turn_id == &started.turn_id),
        "subagent events should carry parent ids: {child_parent_ids:?}"
    );
}

#[tokio::test]
async fn subagent_trace_methods_list_read_and_stream_notifications() {
    let store: Arc<dyn ThreadStoreFactory> = Arc::new(RecordingThreadStoreFactory::default());
    let runtime = subagent_runtime_with_store(
        InProcessDispatcherConfig::default().default_timeout_seconds,
        false,
        Some(store),
    );
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let mut notifications = client.subscribe_notifications();

    let thread_start = start_thread(&client).await;
    let started = start_turn(&client, &thread_start.thread.id, "delegate this").await;

    let trace_notification =
        wait_for_notification(&mut notifications, "turn/subagentTraceCreated", None).await;
    assert_eq!(
        trace_notification.params["summary"]["parent"]["threadId"],
        thread_start.thread.id
    );
    assert_eq!(
        trace_notification.params["summary"]["parent"]["turnId"],
        started.turn_id
    );
    wait_for_event(&mut events, &thread_start.thread.id, "turn.completed").await;

    let traces: SubagentTracesListResult = request(
        &client,
        "turn/subagentTraces/list",
        Some(
            serde_json::to_value(SubagentTracesListParams {
                thread_id: thread_start.thread.id.clone(),
                turn_id: started.turn_id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(traces.traces.len(), 1);
    assert_eq!(traces.traces[0].parent.thread_id, thread_start.thread.id);
    assert_eq!(traces.traces[0].parent.turn_id, started.turn_id);

    let page: SubagentTraceReadResult = request(
        &client,
        "turn/subagentTrace/read",
        Some(
            serde_json::to_value(SubagentTraceReadParams {
                thread_id: thread_start.thread.id,
                trace_id: traces.traces[0].trace_id.clone(),
                offset: 0,
                limit: Some(1),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(page.trace_id, traces.traces[0].trace_id);
    assert_eq!(page.events.len(), 1);
    assert!(page.next_offset.is_none());
}

#[tokio::test]
async fn plan_review_and_hunk_methods_round_trip_through_thread_events() {
    let store: Arc<dyn ThreadStoreFactory> = Arc::new(RecordingThreadStoreFactory::default());
    let workspace =
        std::env::temp_dir().join(format!("roder-plan-review-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "new\n").unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.thread_store_factory(store);
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(workspace.display().to_string()),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::new(runtime.clone()));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let thread_start = start_thread(&client).await;
    let now = OffsetDateTime::now_utc();
    let review = PlanReview {
        id: "review-1".to_string(),
        thread_id: thread_start.thread.id.clone(),
        turn_id: "turn-1".to_string(),
        status: PlanReviewStatus::AwaitingReview,
        title: "Review plan".to_string(),
        markdown: "- edit src/lib.rs".to_string(),
        steps: Vec::new(),
        comments: Vec::new(),
        rewrites: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    runtime
        .emit(roder_api::events::RoderEvent::PlanReviewCreated(
            roder_api::events::PlanReviewCreated {
                review,
                timestamp: now,
            },
        ))
        .await;
    runtime
        .emit(roder_api::events::RoderEvent::HunkRecorded(
            roder_api::events::HunkRecorded {
                hunk: HunkRecord {
                    id: "hunk-1".to_string(),
                    thread_id: thread_start.thread.id.clone(),
                    turn_id: "turn-1".to_string(),
                    path: "src/lib.rs".to_string(),
                    old_start: 1,
                    old_lines: 1,
                    new_start: 1,
                    new_lines: 1,
                    diff: vec![
                        HunkDiffLine {
                            kind: HunkDiffLineKind::Removed,
                            text: "old".to_string(),
                            old_line: Some(1),
                            new_line: None,
                        },
                        HunkDiffLine {
                            kind: HunkDiffLineKind::Added,
                            text: "new".to_string(),
                            old_line: None,
                            new_line: Some(1),
                        },
                    ],
                    tool_call_id: "tool-1".to_string(),
                    tool_name: "apply_patch".to_string(),
                    plan_review_id: Some("review-1".to_string()),
                    plan_step_id: None,
                    timeline_event_id: None,
                    checkpoint_id: None,
                    rollback: HunkRollbackState::Available,
                    reverse_patch: Some("*** Begin Patch\n".to_string()),
                    created_at: now,
                },
                timestamp: now,
            },
        ))
        .await;

    let comment: PlanReviewCommentResult = request(
        &client,
        "plan/review/comment",
        Some(
            serde_json::to_value(PlanReviewCommentParams {
                thread_id: thread_start.thread.id.clone(),
                review_id: "review-1".to_string(),
                anchor: PlanCommentAnchor::Hunk {
                    hunk_id: "hunk-1".to_string(),
                },
                body: "Looks good.".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(comment.comment.body, "Looks good.");
    let notification =
        wait_for_notification(&mut notifications, "plan/reviewCommentAdded", None).await;
    assert_eq!(notification.params["reviewId"], "review-1");

    let _: roder_protocol::PlanReviewApproveResult = request(
        &client,
        "plan/review/approve",
        Some(
            serde_json::to_value(PlanReviewApproveParams {
                thread_id: thread_start.thread.id.clone(),
                review_id: "review-1".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    let read: PlanReviewReadResult = request(
        &client,
        "plan/review/read",
        Some(
            serde_json::to_value(PlanReviewReadParams {
                thread_id: thread_start.thread.id.clone(),
                review_id: "review-1".to_string(),
            })
            .unwrap(),
        ),
    )
    .await;
    let read_review = read.review.unwrap();
    assert_eq!(read_review.status, PlanReviewStatus::Approved);
    assert_eq!(read_review.comments.len(), 1);

    let list: HunkListResult = request(
        &client,
        "hunk/list",
        Some(
            serde_json::to_value(HunkListParams {
                thread_id: thread_start.thread.id.clone(),
                turn_id: Some("turn-1".to_string()),
                review_id: Some("review-1".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(list.hunks.len(), 1);

    let page: HunkReadResult = request(
        &client,
        "hunk/read",
        Some(
            serde_json::to_value(HunkReadParams {
                thread_id: thread_start.thread.id.clone(),
                hunk_id: "hunk-1".to_string(),
                offset: 0,
                limit: Some(1),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(page.page.unwrap().next_offset, Some(1));

    let rollback: HunkRollbackResult = request(
        &client,
        "hunk/rollback",
        Some(
            serde_json::to_value(HunkRollbackParams {
                thread_id: thread_start.thread.id,
                hunk_id: "hunk-1".to_string(),
                confirmed: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(rollback.rolled_back);
    assert_eq!(
        std::fs::read_to_string(workspace.join("src/lib.rs")).unwrap(),
        "old\n"
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn agents_list_returns_public_subagent_summaries() {
    let runtime = subagent_runtime();
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let agents: AgentsListResult = request(&client, "agents/list", None).await;
    assert_eq!(agents.agents.len(), 1);
    assert_eq!(agents.agents[0].agent_type, "explore");
    assert_eq!(agents.agents[0].tools, vec!["echo".to_string()]);

    let serialized = serde_json::to_string(&agents).unwrap();
    assert!(!serialized.contains("SECRET-SYSTEM-PROMPT"));
}

#[tokio::test]
async fn subagent_failed_events_redact_private_agent_material() {
    let runtime = subagent_runtime_with_options(1, true);
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let thread_start = start_thread(&client).await;
    let _: TurnStartResult = start_turn(&client, &thread_start.thread.id, "delegate this").await;

    let failed = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.kind == "subagent.failed" {
            break envelope;
        }
    };
    let serialized = serde_json::to_string(&failed).unwrap();

    assert!(serialized.contains("timeout"));
    assert!(!serialized.contains("SECRET-SYSTEM-PROMPT"));
    assert!(!serialized.contains("Report the relevant finding"));
}

fn subagent_runtime() -> Arc<Runtime> {
    subagent_runtime_with_options(
        InProcessDispatcherConfig::default().default_timeout_seconds,
        false,
    )
}

fn subagent_runtime_with_options(default_timeout_seconds: u64, hang_child: bool) -> Arc<Runtime> {
    subagent_runtime_with_store(default_timeout_seconds, hang_child, None)
}

fn subagent_runtime_with_store(
    default_timeout_seconds: u64,
    hang_child: bool,
    store: Option<Arc<dyn ThreadStoreFactory>>,
) -> Arc<Runtime> {
    let engine = Arc::new(TaskCallingEngine::new(hang_child));
    let mut engines = InferenceEngineRegistry::new();
    engines.insert(engine.clone());
    let mut parent_tools = roder_api::tools::ToolRegistry::default();
    roder_tools::echo_tool_contributor()
        .contribute(&mut parent_tools)
        .unwrap();
    let dispatcher = Arc::new(
        InProcessDispatcher::new(
            InProcessDispatcherConfig {
                default_agent: "explore".to_string(),
                default_provider: Some(PROVIDER_MOCK.to_string()),
                default_model: "mock".to_string(),
                max_depth: 1,
                default_timeout_seconds,
                ..InProcessDispatcherConfig::default()
            },
            vec![SubagentDefinition {
                agent_type: "explore".to_string(),
                description: "Explore the workspace".to_string(),
                tools: vec!["echo".to_string()],
                model: None,
                system_prompt: Some("SECRET-SYSTEM-PROMPT".to_string()),
                permission_mode: SubagentPermissionMode::ReadOnly,
                max_turns: Some(1),
                max_result_chars: Some(1000),
            }],
            engines,
            parent_tools,
        )
        .unwrap(),
    );

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.tool_contributor(roder_tools::echo_tool_contributor());
    if let Some(store) = store {
        builder.thread_store_factory(store);
    }
    builder
        .install(SubagentsExtension::new(dispatcher))
        .unwrap();
    Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap())
}

fn position(kinds: &[String], kind: &str) -> usize {
    kinds
        .iter()
        .position(|candidate| candidate == kind)
        .unwrap_or_else(|| panic!("missing {kind}: {kinds:?}"))
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params,
    };
    let res = client.send_request(req).await;
    assert!(
        res.error.is_none(),
        "RPC error for {method}: {:?}",
        res.error
    );
    serde_json::from_value(res.result.unwrap()).unwrap()
}

async fn wait_for_automation_run(
    client: &LocalAppClient,
    automation_id: &str,
    run_id: &str,
    state: AutomationRunState,
) -> roder_api::automations::AutomationRunSummary {
    for _ in 0..100 {
        let runs: AutomationsRunsResult = request(
            client,
            "automations/runs",
            Some(
                serde_json::to_value(AutomationsRunsParams {
                    automation_id: automation_id.to_string(),
                    state: None,
                    limit: None,
                })
                .unwrap(),
            ),
        )
        .await;
        if let Some(run) = runs
            .runs
            .into_iter()
            .find(|run| run.run_id == run_id && run.state == state)
        {
            return run;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("automation run {run_id} did not reach {state:?}");
}

async fn wait_for_automation_state(
    client: &LocalAppClient,
    automation_id: &str,
    state: AutomationRunState,
    attempts: usize,
) -> roder_api::automations::AutomationRunSummary {
    for _ in 0..attempts {
        let runs: AutomationsRunsResult = request(
            client,
            "automations/runs",
            Some(
                serde_json::to_value(AutomationsRunsParams {
                    automation_id: automation_id.to_string(),
                    state: Some(state),
                    limit: None,
                })
                .unwrap(),
            ),
        )
        .await;
        if let Some(run) = runs.runs.into_iter().find(|run| run.state == state) {
            return run;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("automation {automation_id} did not reach {state:?}");
}

async fn remote_request<T: serde::de::DeserializeOwned>(
    websocket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    id: &str,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    websocket
        .send(Message::Text(
            serde_json::to_string(&JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(id)),
                method: method.to_string(),
                params,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let message = websocket.next().await.unwrap().unwrap();
    let Message::Text(text) = message else {
        panic!("expected text response for {method}");
    };
    let response: roder_protocol::JsonRpcResponse = serde_json::from_str(&text).unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    serde_json::from_value(response.result.unwrap()).unwrap()
}

async fn request_error(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> JsonRpcError {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params,
    };
    client
        .send_request(req)
        .await
        .error
        .unwrap_or_else(|| panic!("expected RPC error for {method}"))
}

fn roadmap_fixture() -> &'static str {
    "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add roadmapping mode.\n**Architecture:** Roadmap documents are first-class state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-app-server/src/server.rs`\n\n## Tasks\n\n- [ ] Add app-server tests\n- [ ] Wire roadmap methods\n\nRun:\n\n```sh\ncargo test -p roder-app-server --test e2e roadmap_methods\n```\n\nAcceptance:\n- App-server roadmap behavior is covered.\n\n## Phase Acceptance\n\n- [ ] App-server works.\n"
}

fn text_input(text: &str) -> Vec<TurnInputItem> {
    vec![TurnInputItem {
        kind: "text".to_string(),
        text: Some(text.to_string()),
        path: None,
        image_url: None,
    }]
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

async fn start_thread(client: &LocalAppClient) -> ThreadStartResult {
    request(
        client,
        "thread/start",
        Some(
            serde_json::to_value(ThreadStartParams {
                model: None,
                model_provider: None,
                cwd: None,
                ephemeral: false,
            })
            .unwrap(),
        ),
    )
    .await
}

async fn start_turn(client: &LocalAppClient, thread_id: &str, text: &str) -> TurnStartResult {
    request(
        client,
        "turn/start",
        Some(
            serde_json::to_value(TurnStartParams {
                thread_id: thread_id.to_string(),
                input: text_input(text),
                prompt: None,
                task_ledger_required: false,
            })
            .unwrap(),
        ),
    )
    .await
}

async fn wait_for_recorded_request(engine: &TaskCallingEngine) -> AgentInferenceRequest {
    for _ in 0..20 {
        if let Some(request) = engine.requests.lock().await.first().cloned() {
            return request;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for recorded inference request");
}

async fn wait_for_image_recorded_request(engine: &ImageRecordingEngine) -> AgentInferenceRequest {
    for _ in 0..20 {
        if let Some(request) = engine.requests.lock().await.first().cloned() {
            return request;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for recorded image inference request");
}

async fn wait_for_event(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
    kind: &str,
) -> roder_api::events::EventEnvelope {
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(thread_id) && envelope.kind == kind {
            return envelope;
        }
    }
}

async fn wait_for_global_event(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    kind: &str,
) -> roder_api::events::EventEnvelope {
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.kind == kind {
            return envelope;
        }
    }
}

async fn wait_for_notification(
    notifications: &mut tokio::sync::broadcast::Receiver<roder_protocol::JsonRpcNotification>,
    method: &str,
    thread_id: Option<&str>,
) -> roder_protocol::JsonRpcNotification {
    loop {
        let notification = tokio::time::timeout(Duration::from_secs(2), notifications.recv())
            .await
            .unwrap()
            .unwrap();
        if notification.method != method {
            continue;
        }
        if let Some(thread_id) = thread_id {
            let notification_thread_id = notification
                .params
                .get("threadId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    notification
                        .params
                        .get("thread")
                        .and_then(|thread| thread.get("id"))
                        .and_then(serde_json::Value::as_str)
                });
            if notification_thread_id != Some(thread_id) {
                continue;
            }
        }
        return notification;
    }
}
