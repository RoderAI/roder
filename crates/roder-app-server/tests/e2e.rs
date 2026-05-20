use base64::Engine;
use futures::{SinkExt, StreamExt, stream};
use roder_api::capabilities::CapabilityDecision;
use roder_api::catalog::{
    PROVIDER_CODEX, PROVIDER_MOCK, PROVIDER_OPENCODE, PROVIDER_OPENCODE_GO, PROVIDER_POOLSIDE,
    PROVIDER_SUPERGROK, PROVIDER_XAI,
};
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
use roder_api::session::{SessionMetadata, SessionStore, SessionStoreFactory, ThreadSnapshot};
use roder_api::subagents::{SubagentDefinition, SubagentPermissionMode};
use roder_api::tasks::TaskState;
use roder_app_server::remote::{
    RemoteServerOptions, RemoteToken, listen_remote_websocket, listen_remote_websocket_controller,
};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{
    PendingPlanExit, Runtime, RuntimeConfig, fake_provider::FakeInferenceEngine,
    media_artifacts::MediaArtifactStore,
};
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
    AgentsListResult, CommandsExpandParams, CommandsExpandResult, CommandsListResult,
    CommandsRunParams, CommandsRunResult, ExtensionsListResult, HunkListParams, HunkListResult,
    HunkReadParams, HunkReadResult, HunkRollbackParams, HunkRollbackResult, InitializeResult,
    JsonRpcError, JsonRpcRequest, MarketplacesAddParams, MarketplacesAddResult,
    MarketplacesListResult, MarketplacesRefreshParams, MarketplacesRefreshResult,
    MarketplacesRemoveParams, MarketplacesRemoveResult, MarketplacesSearchParams,
    MarketplacesSearchResult, MediaAttachToTurnParams, MediaAttachToTurnResult, MediaDeleteParams,
    MediaDeleteResult, MediaListParams, MediaListResult, MediaReadParams, MediaReadResult,
    MediaThumbnailParams, MediaThumbnailResult, MemoryDeleteParams, MemoryDeleteResult,
    MemoryListParams, MemoryListResult, MemoryProviderListResult, MemoryQueryParams,
    MemoryQueryResult, MemoryReadParams, MemoryReadResult, MemoryRecallPreviewParams,
    MemoryRecallPreviewResult, MemorySaveParams, MemorySaveResult, MemoryUpdateParams,
    PlanReviewApproveParams, PlanReviewCommentParams, PlanReviewCommentResult,
    PlanReviewReadParams, PlanReviewReadResult, PluginDisableParams, PluginDisableResult,
    PluginInstallAllVariantsParams, PluginInstallAllVariantsResult, PluginInstallParams,
    PluginInstallResult, PluginListInstalledResult, PluginPreviewInstallParams,
    PluginPreviewInstallResult, PluginUninstallParams, PluginUninstallResult, ProviderAuthResult,
    ProviderSelectParams, ProviderSelectResult, ProvidersListResult, RunnersDeleteResult,
    RunnersListResult, RunnersSelectParams, RunnersSelectResult, RunnersSessionResult,
    SessionExitPlanParams, SessionExitPlanResult, SessionGetResult, SessionResolveApprovalParams,
    SessionResolveApprovalResult, SessionResolveUserInputParams, SessionResolveUserInputResult,
    SessionSetModeParams, SessionSetModeResult, SettingsGetResult, SettingsSetDefaultModeParams,
    SettingsSetDefaultModeResult, SettingsSetWebSearchParams, SettingsSetWebSearchResult,
    SubagentTraceReadParams, SubagentTraceReadResult, SubagentTracesListParams,
    SubagentTracesListResult, TasksGetParams, TasksGetResult, TasksListResult, TasksSubmitParams,
    TasksSubmitResult, TeamCleanupParams, TeamCleanupResult, TeamListParams, TeamListResult,
    TeamMemberInterruptParams, TeamMemberInterruptResult, TeamMemberMessageParams,
    TeamMemberMessageResult, TeamMemberStartParams, TeamMemberStartResult, TeamReadParams,
    TeamReadResult, TeamStartMemberParams, TeamStartParams, TeamStartResult, ThreadArchiveParams,
    ThreadArchiveResult, ThreadListParams, ThreadListResult, ThreadReadResult, ThreadStartParams,
    ThreadStartResult, ToolCallParams, ToolCallResult, ToolsListResult, TurnInputItem,
    TurnInterruptParams, TurnStartParams, TurnStartResult, TurnSteerParams, TurnSteerResult,
    WorkflowEnableParams, WorkflowEnableResult, WorkflowPreviewParams, WorkflowPreviewResult,
    WorkflowScanParams, WorkflowScanResult,
};
use std::collections::HashMap;
use std::sync::Arc;
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

struct PendingEngine;

struct ImageRecordingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct FailingSessionStoreFactory;

struct FailingSessionStore;

#[derive(Clone, Default)]
struct RecordingSessionStoreFactory {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

struct RecordingSessionStore {
    snapshots: Arc<Mutex<HashMap<String, ThreadSnapshot>>>,
}

impl SessionStoreFactory for FailingSessionStoreFactory {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "failing".to_string()
    }

    fn create(&self) -> Arc<dyn SessionStore> {
        Arc::new(FailingSessionStore)
    }
}

#[async_trait::async_trait]
impl SessionStore for FailingSessionStore {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "failing".to_string()
    }

    async fn create_session(&self, metadata: SessionMetadata) -> anyhow::Result<SessionMetadata> {
        Ok(metadata)
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>> {
        anyhow::bail!(
            "parse session metadata /tmp/roder/sessions/bad/metadata.json: trailing characters at line 1 column 450"
        );
    }

    async fn load_session(
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
        _item: &roder_api::conversation::TurnItem,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl SessionStoreFactory for RecordingSessionStoreFactory {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "recording".to_string()
    }

    fn create(&self) -> Arc<dyn SessionStore> {
        Arc::new(RecordingSessionStore {
            snapshots: Arc::clone(&self.snapshots),
        })
    }
}

#[async_trait::async_trait]
impl SessionStore for RecordingSessionStore {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "recording".to_string()
    }

    async fn create_session(&self, metadata: SessionMetadata) -> anyhow::Result<SessionMetadata> {
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

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>> {
        Ok(self
            .snapshots
            .lock()
            .await
            .values()
            .filter_map(|snapshot| snapshot.metadata.clone())
            .collect())
    }

    async fn load_session(
        &self,
        thread_id: &roder_api::events::ThreadId,
    ) -> anyhow::Result<Option<ThreadSnapshot>> {
        Ok(self.snapshots.lock().await.get(thread_id).cloned())
    }

    async fn archive_session(
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
            snapshot.events.push(envelope.clone());
        }
        Ok(())
    }

    async fn append_turn_item(
        &self,
        thread_id: &roder_api::events::ThreadId,
        turn_id: &roder_api::events::TurnId,
        item: &roder_api::conversation::TurnItem,
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
                snapshot.turns.push(roder_api::session::TurnRecord {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    items: vec![item.clone()],
                    created_at: OffsetDateTime::now_utc(),
                    completed_at: None,
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
    assert_eq!(tools.tools.len(), 1);
    assert_eq!(tools.tools[0].name, "echo");

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

    let session = start_thread(&client).await;
    assert_eq!(session.model_provider, PROVIDER_MOCK);
    assert_eq!(session.model, "alternate-mock-model");
    assert!(!session.thread.id.is_empty());

    let sessions: ThreadListResult = request(
        &client,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
    )
    .await;
    assert!(
        sessions
            .data
            .iter()
            .any(|thread| thread.id == session.thread.id)
    );

    let started = start_turn(&client, &session.thread.id, "Hello").await;
    assert!(!started.turn_id.is_empty());

    let mut kinds = Vec::new();
    for _ in 0..12 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(&session.thread.id) {
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
async fn providers_select_updates_desktop_thread_model_for_next_turn() {
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
async fn desktop_turn_uses_thread_cwd_for_workspace_tools() {
    let root = std::env::temp_dir().join(format!(
        "roder-thread-cwd-e2e-{}",
        uuid::Uuid::new_v4()
    ));
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
        .conversation
        .iter()
        .rev()
        .find_map(|item| match item {
            roder_api::conversation::ConversationItem::UserMessage(message) => Some(message),
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
    assert!(tokio_tungstenite::connect_async(origin_request)
        .await
        .is_err());

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
    builder.session_store_factory(Arc::new(FailingSessionStoreFactory));
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
    assert!(error.message.contains("parse session metadata"));
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

    let session = start_thread(&client).await;
    let started = start_turn(&client, &session.thread.id, "start").await;
    wait_for_event(&mut events, &session.thread.id, "turn.started").await;

    let steered: TurnSteerResult = request(
        &client,
        "turn/steer",
        Some(
            serde_json::to_value(TurnSteerParams {
                thread_id: session.thread.id.clone(),
                expected_turn_id: started.turn_id.clone(),
                input: text_input("change direction"),
                prompt: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(steered.turn_id, started.turn_id);

    let event = wait_for_event(&mut events, &session.thread.id, "turn.steered").await;
    assert_eq!(event.turn_id.as_deref(), Some(started.turn_id.as_str()));

    let _: roder_protocol::TurnInterruptResult = request(
        &client,
        "turn/interrupt",
        Some(
            serde_json::to_value(TurnInterruptParams {
                thread_id: session.thread.id,
                turn_id: Some(started.turn_id),
            })
            .unwrap(),
        ),
    )
    .await;
}

#[tokio::test]
async fn desktop_contract_methods_support_desktop_startup_contract() {
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
async fn thread_archive_removes_session_from_desktop_thread_list() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.session_store_factory(Arc::new(RecordingSessionStoreFactory::default()));
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

    let sessions: ThreadListResult = request(
        &client,
        "thread/list",
        Some(serde_json::to_value(ThreadListParams { limit: None }).unwrap()),
    )
    .await;
    assert!(
        !sessions
            .data
            .iter()
            .any(|thread| thread.id == archived.thread_id)
    );
}

#[tokio::test]
async fn desktop_contract_turn_methods_and_notifications_match_desktop_contract() {
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
async fn desktop_contract_turn_interrupt_uses_active_turn_when_turn_id_is_omitted() {
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
async fn desktop_notifications_surface_tool_approval_requests_and_resolution() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(ApprovalRequiredEngine {
        calls: Mutex::new(0),
    }));
    builder.tool_contributor(roder_tools::builtin_coding_tools_contributor(".").unwrap());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut notifications = client.subscribe_notifications();

    let session = start_thread(&client).await;
    let _started = start_turn(&client, &session.thread.id, "what branch are you on?").await;

    let approval = wait_for_notification(
        &mut notifications,
        "session/approvalRequested",
        Some(&session.thread.id),
    )
    .await;
    assert_eq!(approval.params["turnId"].as_str().is_some(), true);
    assert_eq!(approval.params["approvalId"], "approval-shell-1");
    assert_eq!(approval.params["toolId"], "approval-shell-1");
    assert_eq!(approval.params["toolName"], "shell");

    let waiting_status = wait_for_notification(
        &mut notifications,
        "thread/status/changed",
        Some(&session.thread.id),
    )
    .await;
    assert_eq!(waiting_status.params["status"]["type"], "running");
    assert_eq!(
        waiting_status.params["status"]["activeFlags"],
        serde_json::json!(["approvalRequired"])
    );

    let resolved: SessionResolveApprovalResult = request(
        &client,
        "session/resolve_approval",
        Some(
            serde_json::to_value(SessionResolveApprovalParams {
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
        "session/approvalResolved",
        Some(&session.thread.id),
    )
    .await;
    assert_eq!(
        resolved_notification.params["approvalId"],
        "approval-shell-1"
    );
    assert_eq!(resolved_notification.params["approved"], false);
}

#[tokio::test]
async fn desktop_contract_fs_and_command_methods_match_desktop_contract() {
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
async fn tools_list_exposes_default_coding_tools() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let tools: ToolsListResult = request(&client, "tools/list", None).await;
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
async fn commands_list_and_expand_are_deterministic() {
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
}

#[tokio::test]
async fn commands_run_expands_and_starts_turn() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let session = start_thread(&client).await;
    let result: CommandsRunResult = request(
        &client,
        "commands/run",
        Some(
            serde_json::to_value(CommandsRunParams {
                thread_id: session.thread.id.clone(),
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
    wait_for_event(&mut events, &session.thread.id, "turn.completed").await;
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
async fn tools_call_can_create_and_get_goal() {
    let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
    let runtime = Arc::new(Runtime::new(registry, Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let session = start_thread(&client).await;
    let created: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: session.thread.id.clone(),
                tool_name: "create_goal".to_string(),
                arguments: serde_json::json!({
                    "objective": "Ship slash goal",
                    "replace": true,
                }),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!created.is_error, "create_goal failed: {created:?}");
    assert_eq!(created.text, "Goal active: Ship slash goal");

    let current: ToolCallResult = request(
        &client,
        "tools/call",
        Some(
            serde_json::to_value(ToolCallParams {
                thread_id: session.thread.id,
                tool_name: "get_goal".to_string(),
                arguments: serde_json::json!({}),
            })
            .unwrap(),
        ),
    )
    .await;

    assert!(!current.is_error, "get_goal failed: {current:?}");
    assert_eq!(current.text, "Goal active: Ship slash goal");
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

    let session = start_thread(&client).await;
    let _started = start_turn(&client, &session.thread.id, "ask me").await;

    let requested_notification = wait_for_notification(
        &mut notifications,
        "session/userInputRequested",
        Some(&session.thread.id),
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

    let resolved: SessionResolveUserInputResult = request(
        &client,
        "session/resolve_user_input",
        Some(
            serde_json::to_value(SessionResolveUserInputParams {
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
        "session/userInputResolved",
        Some(&session.thread.id),
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
async fn session_policy_mode_can_be_set_and_observed() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let state: SessionGetResult = request(&client, "session/get", None).await;
    assert_eq!(state.mode, PolicyMode::Default);

    let changed: SessionSetModeResult = request(
        &client,
        "session/set_mode",
        Some(
            serde_json::to_value(SessionSetModeParams {
                mode: PolicyMode::Plan,
                reason: Some("test".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(changed.mode, PolicyMode::Plan);

    let state: SessionGetResult = request(&client, "session/get", None).await;
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
async fn session_exit_plan_resolves_pending_request() {
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
        "session/planExitRequested",
        Some("thread-plan"),
    )
    .await;
    assert_eq!(requested_notification.params["requestId"], "exit-plan-1");
    assert_eq!(requested_notification.params["targetMode"], "default");
    assert_eq!(
        requested_notification.params["planSummary"],
        "Implement approved edits"
    );

    let state: SessionGetResult = request(&client, "session/get", None).await;
    assert_eq!(
        state
            .pending_plan_exit
            .as_ref()
            .map(|pending| pending.request_id.as_str()),
        Some("exit-plan-1")
    );

    let resolved: SessionExitPlanResult = request(
        &client,
        "session/exit_plan",
        Some(
            serde_json::to_value(SessionExitPlanParams {
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
        "session/planExitResolved",
        Some("thread-plan"),
    )
    .await;
    assert_eq!(resolved_notification.params["requestId"], "exit-plan-1");
    assert_eq!(resolved_notification.params["approved"], true);
    assert_eq!(resolved_notification.params["resolvedMode"], "default");

    let state: SessionGetResult = request(&client, "session/get", None).await;
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
async fn session_exit_plan_timeout_rejects_late_approval() {
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

    let resolved: SessionExitPlanResult = request(
        &client,
        "session/exit_plan",
        Some(
            serde_json::to_value(SessionExitPlanParams {
                request_id: "exit-plan-expired".to_string(),
                approved: true,
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(resolved.resolved);
    assert_eq!(resolved.mode, PolicyMode::Plan);

    let state: SessionGetResult = request(&client, "session/get", None).await;
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

    let session = start_thread(&client).await;
    let started = start_turn(&client, &session.thread.id, "delegate this").await;

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
            .all(|(thread_id, turn_id)| thread_id == &session.thread.id
                && turn_id == &started.turn_id),
        "subagent events should carry parent ids: {child_parent_ids:?}"
    );
}

#[tokio::test]
async fn subagent_trace_methods_list_read_and_stream_notifications() {
    let store: Arc<dyn SessionStoreFactory> = Arc::new(RecordingSessionStoreFactory::default());
    let runtime = subagent_runtime_with_store(
        InProcessDispatcherConfig::default().default_timeout_seconds,
        false,
        Some(store),
    );
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();
    let mut notifications = client.subscribe_notifications();

    let session = start_thread(&client).await;
    let started = start_turn(&client, &session.thread.id, "delegate this").await;

    let trace_notification =
        wait_for_notification(&mut notifications, "turn/subagentTraceCreated", None).await;
    assert_eq!(
        trace_notification.params["summary"]["parent"]["threadId"],
        session.thread.id
    );
    assert_eq!(
        trace_notification.params["summary"]["parent"]["turnId"],
        started.turn_id
    );
    wait_for_event(&mut events, &session.thread.id, "turn.completed").await;

    let traces: SubagentTracesListResult = request(
        &client,
        "turn/subagentTraces/list",
        Some(
            serde_json::to_value(SubagentTracesListParams {
                thread_id: session.thread.id.clone(),
                turn_id: started.turn_id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(traces.traces.len(), 1);
    assert_eq!(traces.traces[0].parent.thread_id, session.thread.id);
    assert_eq!(traces.traces[0].parent.turn_id, started.turn_id);

    let page: SubagentTraceReadResult = request(
        &client,
        "turn/subagentTrace/read",
        Some(
            serde_json::to_value(SubagentTraceReadParams {
                thread_id: session.thread.id,
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
async fn plan_review_and_hunk_methods_round_trip_through_session_events() {
    let store: Arc<dyn SessionStoreFactory> = Arc::new(RecordingSessionStoreFactory::default());
    let workspace =
        std::env::temp_dir().join(format!("roder-plan-review-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "new\n").unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.session_store_factory(store);
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

    let session = start_thread(&client).await;
    let now = OffsetDateTime::now_utc();
    let review = PlanReview {
        id: "review-1".to_string(),
        thread_id: session.thread.id.clone(),
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
                    thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id.clone(),
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
                thread_id: session.thread.id,
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

    let session = start_thread(&client).await;
    let _: TurnStartResult = start_turn(&client, &session.thread.id, "delegate this").await;

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
    store: Option<Arc<dyn SessionStoreFactory>>,
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
        builder.session_store_factory(store);
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

fn text_input(text: &str) -> Vec<TurnInputItem> {
    vec![TurnInputItem {
        kind: "text".to_string(),
        text: Some(text.to_string()),
        path: None,
        image_url: None,
    }]
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
