use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::conversation::{ConversationItem, InputImage};
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::*;
use roder_api::policy_mode::PolicyMode;
use roder_api::subagents::{SubagentDefinition, SubagentPermissionMode};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{PendingPlanExit, Runtime, fake_provider::FakeInferenceEngine};
use roder_ext_subagents::{
    InProcessDispatcher, InProcessDispatcherConfig, InferenceEngineRegistry, SubagentsExtension,
};
use roder_extension_host::{
    DefaultRegistryConfig, DefaultWebSearchConfig, DefaultWebSearchProviderConfig,
    build_default_registry,
};
use roder_protocol::{
    AgentsListResult, CreateSessionResult, ExtensionsListResult, InterruptTurnParams,
    JsonRpcRequest, ProviderSelectParams, ProviderSelectResult, ProvidersListResult,
    SessionExitPlanParams, SessionExitPlanResult, SessionGetResult, SessionResolveUserInputParams,
    SessionResolveUserInputResult, SessionSetModeParams, SessionSetModeResult, SessionsListResult,
    SettingsGetResult, SettingsSetDefaultModeParams, SettingsSetDefaultModeResult,
    SettingsSetWebSearchParams, SettingsSetWebSearchResult, StartTurnParams, StartTurnResult,
    SteerTurnParams, SteerTurnResult, SystemStatusResult, ToolsListResult,
};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::Mutex;

struct TaskCallingEngine {
    hang_child: bool,
    parent_calls: Mutex<usize>,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct PendingEngine;

struct ImageCaptureEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct UserInputEngine {
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
impl InferenceEngine for ImageCaptureEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            image_input: true,
            ..InferenceCapabilities::text_only()
        }
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
        Ok(Box::pin(stream::iter([
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "ok".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
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

    let status: SystemStatusResult = request(&client, "system/status", None).await;
    assert_eq!(status.provider, PROVIDER_MOCK);
    assert_eq!(status.model, "mock");
    assert_eq!(status.web_search.mode, HostedWebSearchMode::Cached);

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
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(selected.provider, PROVIDER_MOCK);
    assert_eq!(selected.model, "alternate-mock-model");
    assert_eq!(selected.reasoning, "none");

    let status: SystemStatusResult = request(&client, "system/status", None).await;
    assert_eq!(status.provider, PROVIDER_MOCK);
    assert_eq!(status.model, "alternate-mock-model");
    assert_eq!(status.reasoning, "none");

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

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    assert_eq!(session.provider, PROVIDER_MOCK);
    assert_eq!(session.model, "alternate-mock-model");
    assert!(!session.thread_id.is_empty());

    let sessions: SessionsListResult = request(&client, "sessions/list", None).await;
    assert!(sessions.sessions.is_empty());

    let params = StartTurnParams {
        thread_id: session.thread_id.clone(),
        message: "Hello".to_string(),
        images: Vec::new(),
        provider_override: None,
        model_override: None,
    };
    let started: StartTurnResult = request(
        &client,
        "turns/start",
        Some(serde_json::to_value(params).unwrap()),
    )
    .await;
    assert!(!started.turn_id.is_empty());

    let mut kinds = Vec::new();
    for _ in 0..12 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if envelope.thread_id.as_deref() == Some(&session.thread_id) {
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
async fn turns_steer_requires_active_turn() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("turns/steer")),
            method: "turns/steer".to_string(),
            params: Some(
                serde_json::to_value(SteerTurnParams {
                    thread_id: "thread_missing".to_string(),
                    turn_id: "turn_missing".to_string(),
                    message: "change direction".to_string(),
                    images: Vec::new(),
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
async fn turns_start_preserves_image_payloads_for_provider_request() {
    let engine = Arc::new(ImageCaptureEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let _: StartTurnResult = request(
        &client,
        "turns/start",
        Some(
            serde_json::to_value(StartTurnParams {
                thread_id: session.thread_id.clone(),
                message: "what is this?".to_string(),
                images: vec![InputImage {
                    image_url: "data:image/png;base64,YWJj".to_string(),
                }],
                provider_override: None,
                model_override: None,
            })
            .unwrap(),
        ),
    )
    .await;
    wait_for_event(&mut events, &session.thread_id, "turn.completed").await;

    let requests = engine.requests.lock().await;
    assert_eq!(requests.len(), 1);
    let user_message = requests[0]
        .conversation
        .iter()
        .find_map(|item| match item {
            ConversationItem::UserMessage(message) if message.text == "what is this?" => {
                Some(message)
            }
            _ => None,
        })
        .expect("missing user message");
    assert_eq!(user_message.images.len(), 1);
    assert_eq!(
        user_message.images[0].image_url,
        "data:image/png;base64,YWJj"
    );
}

#[tokio::test]
async fn turns_steer_accepts_active_turn() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let started: StartTurnResult = request(
        &client,
        "turns/start",
        Some(
            serde_json::to_value(StartTurnParams {
                thread_id: session.thread_id.clone(),
                message: "start".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
            })
            .unwrap(),
        ),
    )
    .await;
    wait_for_event(&mut events, &session.thread_id, "turn.started").await;

    let steered: SteerTurnResult = request(
        &client,
        "turns/steer",
        Some(
            serde_json::to_value(SteerTurnParams {
                thread_id: session.thread_id.clone(),
                turn_id: started.turn_id.clone(),
                message: "change direction".to_string(),
                images: Vec::new(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(steered.turn_id, started.turn_id);

    let event = wait_for_event(&mut events, &session.thread_id, "turn.steered").await;
    assert_eq!(event.turn_id.as_deref(), Some(started.turn_id.as_str()));

    let _: serde_json::Value = request(
        &client,
        "turns/interrupt",
        Some(
            serde_json::to_value(InterruptTurnParams {
                thread_id: session.thread_id,
                turn_id: started.turn_id,
            })
            .unwrap(),
        ),
    )
    .await;
}

#[tokio::test]
async fn desktop_protocol_methods_are_not_supported() {
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);

    for method in [
        "initialize",
        "thread/start",
        "thread/list",
        "thread/read",
        "turn/start",
        "turn/steer",
        "turn/interrupt",
        "model/list",
    ] {
        let response = client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(method)),
                method: method.to_string(),
                params: None,
            })
            .await;
        let error = response
            .error
            .expect("old Desktop method should be rejected");
        assert_eq!(error.code, -32601, "{method}");
    }
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

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let _started: StartTurnResult = request(
        &client,
        "turns/start",
        Some(
            serde_json::to_value(StartTurnParams {
                thread_id: session.thread_id,
                message: "ask me".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
            })
            .unwrap(),
        ),
    )
    .await;

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

    let status: SystemStatusResult = request(&client, "system/status", None).await;
    assert_eq!(status.web_search.mode, HostedWebSearchMode::Live);
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
    runtime
        .record_pending_plan_exit(PendingPlanExit::new(
            "thread-plan".to_string(),
            "turn-plan".to_string(),
            "exit-plan-1".to_string(),
            PolicyMode::Default,
            Some("Implement approved edits".to_string()),
        ))
        .await;
    let server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(server);
    let mut events = client.subscribe_events();

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

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let started: StartTurnResult = request(
        &client,
        "turns/start",
        Some(
            serde_json::to_value(StartTurnParams {
                thread_id: session.thread_id.clone(),
                message: "delegate this".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
            })
            .unwrap(),
        ),
    )
    .await;

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
            _ => {}
        }
        if envelope.kind == "turn.completed" {
            break;
        }
    }

    let subagent_started = position(&kinds, "subagent.started");
    let subagent_completed = position(&kinds, "subagent.completed");
    let tool_completed = position(&kinds, "tool.call_completed");
    assert!(subagent_started < subagent_completed, "{kinds:?}");
    assert!(subagent_completed < tool_completed, "{kinds:?}");
    assert!(
        position(&kinds, "turn.completed") > tool_completed,
        "{kinds:?}"
    );
    assert!(
        child_parent_ids
            .iter()
            .all(|(thread_id, turn_id)| thread_id == &session.thread_id
                && turn_id == &started.turn_id),
        "subagent events should carry parent ids: {child_parent_ids:?}"
    );
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

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let _: StartTurnResult = request(
        &client,
        "turns/start",
        Some(
            serde_json::to_value(StartTurnParams {
                thread_id: session.thread_id,
                message: "delegate this".to_string(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
            })
            .unwrap(),
        ),
    )
    .await;

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
