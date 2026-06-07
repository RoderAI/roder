use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;
use tokio::sync::Notify;

struct ToolLoopEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
    tool_rounds: usize,
}

struct ParallelToolLoopEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct AgentControlEngine {
    thread_ids: Mutex<Vec<String>>,
    tool_names_by_thread: Mutex<Vec<(String, Vec<String>)>>,
}

struct EchoContributor;

impl ToolContributor for EchoContributor {
    fn id(&self) -> ToolProviderId {
        "test-echo".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(EchoTool))
    }
}

struct EchoTool;

#[async_trait::async_trait]
impl ToolExecutor for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "Echo text".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let text = call
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: text.clone(),
            data: json!({ "text": text }),
            is_error: false,
        })
    }
}

struct BlockingEchoContributor {
    release: Arc<Notify>,
}

impl ToolContributor for BlockingEchoContributor {
    fn id(&self) -> ToolProviderId {
        "test-blocking-echo".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(BlockingEchoTool {
            release: self.release.clone(),
        }))
    }
}

struct BlockingEchoTool {
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl ToolExecutor for BlockingEchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".to_string(),
            description: "Echo text after release".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        self.release.notified().await;
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "from blocked tool".to_string(),
            data: json!({ "text": "from blocked tool" }),
            is_error: false,
        })
    }
}

struct DuplicateWebSearchContributor(&'static str);

impl ToolContributor for DuplicateWebSearchContributor {
    fn id(&self) -> ToolProviderId {
        self.0.to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(WebSearchTool))
    }
}

struct EditSurfaceContributor;

impl ToolContributor for EditSurfaceContributor {
    fn id(&self) -> ToolProviderId {
        "test-edit-surface".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        for name in [
            "apply_patch",
            "write_file",
            "edit",
            "multi_edit",
            "read_file",
        ] {
            registry.register(Arc::new(NamedTool(name)))?;
        }
        Ok(())
    }
}

struct NamedTool(&'static str);

#[async_trait::async_trait]
impl ToolExecutor for NamedTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.0.to_string(),
            description: format!("{} tool", self.0),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: String::new(),
            data: json!({}),
            is_error: false,
        })
    }
}

struct FailingContributor;

impl ToolContributor for FailingContributor {
    fn id(&self) -> ToolProviderId {
        "test-failing".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(FailingTool))
    }
}

struct FailingTool;

#[async_trait::async_trait]
impl ToolExecutor for FailingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "failing_tool".to_string(),
            description: "Always fails".to_string(),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        _call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        anyhow::bail!("path does not exist: crates/roder-tui/src/main.rs")
    }
}

struct WebSearchTool;

#[async_trait::async_trait]
impl ToolExecutor for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".to_string(),
            description: "Search the web.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: String::new(),
            data: json!({}),
            is_error: false,
        })
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ToolLoopEngine {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let turn = requests.len();
        drop(requests);
        let events = if turn <= self.tool_rounds {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: format!("call_{turn}"),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"from tool"}"#.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_1".to_string()),
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "final after tool".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp_2".to_string()),
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for ParallelToolLoopEngine {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let turn = requests.len();
        drop(requests);
        let events = if turn == 1 {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"one"}"#.to_string(),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_2".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"two"}"#.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_tools".to_string()),
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp_done".to_string()),
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for AgentControlEngine {
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
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let is_parent = ctx.thread_id == "thread-agent-control";
        let has_result = |name: &str| {
            request.transcript.iter().any(|item| {
                matches!(
                    item,
                    TranscriptItem::ToolResult(result)
                        if result.name.as_deref() == Some(name)
                )
            })
        };
        let mut thread_ids = self.thread_ids.lock().unwrap();
        thread_ids.push(ctx.thread_id.to_string());
        drop(thread_ids);
        self.tool_names_by_thread.lock().unwrap().push((
            ctx.thread_id.to_string(),
            request.tools.into_iter().map(|tool| tool.name).collect(),
        ));
        let events = if is_parent && !has_result("spawn_agent") {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_spawn".to_string(),
                    name: "spawn_agent".to_string(),
                    arguments: json!({
                        "task_name": "reviewer",
                        "message": "review the code"
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_spawn".to_string()),
                })),
            ]
        } else if is_parent && !has_result("list_agents") {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_list".to_string(),
                    name: "list_agents".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_list".to_string()),
                })),
            ]
        } else if is_parent && !has_result("send_message") {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_send".to_string(),
                    name: "send_message".to_string(),
                    arguments: json!({
                        "target": "reviewer",
                        "message": "add one more detail"
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_wait".to_string(),
                    name: "wait_agent".to_string(),
                    arguments: json!({
                        "target": "reviewer",
                        "timeout_ms": 100
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_close".to_string(),
                    name: "close_agent".to_string(),
                    arguments: json!({
                        "target": "reviewer"
                    })
                    .to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_followup_batch".to_string()),
                })),
            ]
        } else if is_parent {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "parent done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp_parent".to_string()),
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "child done".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp_child".to_string()),
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

struct ErrorRecoveringEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct PhasePreservingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

struct FailingStreamStartEngine;

#[async_trait::async_trait]
impl InferenceEngine for ErrorRecoveringEngine {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let has_tool_error = requests.last().unwrap().transcript.iter().any(|item| {
            matches!(
                item,
                TranscriptItem::ToolResult(result)
                    if result.is_error
                        && result.result.contains("path does not exist")
            )
        });
        drop(requests);

        let events = if has_tool_error {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "recovered".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: Some("resp_recovered".to_string()),
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_failed_read".to_string(),
                    name: "failing_tool".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_tool".to_string()),
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for PhasePreservingEngine {
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
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let turn = requests.len();
        drop(requests);

        let events = if turn == 1 {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "I will inspect first.".to_string(),
                    phase: Some("commentary".to_string()),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"from tool"}"#.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ]
        } else {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "Done.".to_string(),
                    phase: Some("final_answer".to_string()),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

#[async_trait::async_trait]
impl InferenceEngine for FailingStreamStartEngine {
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
        anyhow::bail!("input exceeds the context window")
    }
}

#[tokio::test]
async fn run_turn_continues_after_tool_result() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 1,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EchoContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "gpt-5.5".to_string(),
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: Some(10_000),
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_1".to_string(),
            message: "echo please".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some("thread_1") {
            break;
        }
    }

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].reasoning.level.as_deref(), Some("low"));
    assert!(
        requests[0]
            .instructions
            .system
            .as_deref()
            .is_some_and(|text| text.contains("You are Roder")
                && text.contains("send a brief preamble message")),
        "first request should include the Roder system prompt with intermediary message guidance"
    );
    assert!(
        requests[1]
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::ToolResult(result) if result.result == "from tool")),
        "second request should include the tool result: {:?}",
        requests[1].transcript
    );
}

#[tokio::test]
async fn run_turn_executes_parallel_tool_call_batch_concurrently() {
    let engine = Arc::new(ParallelToolLoopEngine {
        requests: Mutex::new(Vec::new()),
    });
    let release = Arc::new(Notify::new());
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(BlockingEchoContributor {
        release: release.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "gpt-5.5".to_string(),
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_parallel".to_string(),
            message: "run tools".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let started_two_tools = tokio::time::timeout(Duration::from_millis(300), async {
        let mut started = Vec::new();
        while started.len() < 2 {
            let event = events.recv().await.unwrap();
            if let roder_api::events::RoderEvent::ToolCallStarted(tool) = event.event {
                started.push(tool.tool_id);
            }
        }
        started
    })
    .await;
    if started_two_tools.is_err() {
        release.notify_waiters();
        panic!("runtime did not start both tool calls before the first blocked");
    }
    let started = started_two_tools.unwrap();
    assert_eq!(started, vec!["call_1".to_string(), "call_2".to_string()]);

    release.notify_waiters();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some("thread_parallel") {
            break;
        }
    }

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].runtime.parallel_tool_calls, Some(true));
    let result_count = requests[1]
        .transcript
        .iter()
        .filter(|item| matches!(item, TranscriptItem::ToolResult(_)))
        .count();
    assert_eq!(result_count, 2);
}

#[tokio::test]
async fn provider_start_errors_are_emitted_for_the_active_thread() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FailingStreamStartEngine));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_context_error".to_string(),
            message: "short prompt".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::TurnFailed(failed) = event.event {
            assert_eq!(failed.thread_id, "thread_context_error");
            assert!(failed.error.contains("context window"));
            break;
        }
    }
}

#[tokio::test]
async fn commentary_phase_messages_are_preserved_for_next_provider_request() {
    let engine = Arc::new(PhasePreservingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EchoContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_phase_messages".to_string(),
            message: "inspect".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_phase_messages").await;

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::AssistantMessage(message)
                if message.text == "I will inspect first."
                    && message.phase.as_deref() == Some("commentary")
        )),
        "second request should preserve commentary assistant message: {:?}",
        requests[1].transcript
    );
}

#[tokio::test]
async fn steer_turn_is_included_in_next_provider_request() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 1,
    });
    let release_tool = Arc::new(Notify::new());
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(BlockingEchoContributor {
        release: release_tool.clone(),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();

    let turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_steer".to_string(),
            message: "start".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_kind(&mut events, "thread_steer", "tool.call_requested").await;

    runtime
        .steer_turn(
            "thread_steer".to_string(),
            turn_id,
            "use the new constraint".to_string(),
            Vec::new(),
        )
        .await
        .unwrap();
    release_tool.notify_waiters();
    wait_for_completed(&mut events, "thread_steer").await;

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].transcript.iter().any(|item| {
            matches!(item, TranscriptItem::UserMessage(message) if message.text == "use the new constraint")
        }),
        "second request should include steer message: {:?}",
        requests[1].transcript
    );
}

#[tokio::test]
async fn runtime_advertises_apply_patch_for_default_edit_models() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 0,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EditSurfaceContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "gpt-5.5".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_patch_tools".to_string(),
            message: "patch please".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_patch_tools").await;

    let requests = engine.requests.lock().unwrap();
    let names = request_tool_names(&requests[0]);
    assert!(names.contains(&"apply_patch".to_string()), "{names:?}");
    for excluded in ["write_file", "edit", "multi_edit"] {
        assert!(!names.contains(&excluded.to_string()), "{names:?}");
    }
    assert!(names.contains(&"read_file".to_string()), "{names:?}");
}

#[tokio::test]
async fn runtime_keeps_apply_patch_for_edit_profile_models() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 0,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EditSurfaceContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "edit-model".to_string(),
                model_edit_tools: std::collections::HashMap::from([(
                    "edit-model".to_string(),
                    "edit".to_string(),
                )]),
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_edit_tools".to_string(),
            message: "edit please".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_edit_tools").await;

    let requests = engine.requests.lock().unwrap();
    let names = request_tool_names(&requests[0]);
    for included in ["apply_patch", "write_file", "edit", "multi_edit", "read_file"] {
        assert!(names.contains(&included.to_string()), "{names:?}");
    }
}

#[tokio::test]
async fn runtime_uses_custom_model_edit_tool_override() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 0,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EditSurfaceContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "custom-model".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::from([(
                    "custom-model".to_string(),
                    "patch".to_string(),
                )]),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );

    let names = runtime
        .tool_specs()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"apply_patch".to_string()), "{names:?}");
    assert!(!names.contains(&"edit".to_string()), "{names:?}");
    assert!(!names.contains(&"multi_edit".to_string()), "{names:?}");
    assert!(!names.contains(&"write_file".to_string()), "{names:?}");
}

#[tokio::test]
async fn model_can_spawn_long_lived_subagent_with_agent_control_tool() {
    let engine = Arc::new(AgentControlEngine {
        thread_ids: Mutex::new(Vec::new()),
        tool_names_by_thread: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
                team_data_dir: Some(std::env::temp_dir().join(format!(
                    "roder-agent-control-tools-{}",
                    uuid::Uuid::new_v4()
                ))),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut rx = runtime.subscribe_events();
    let _turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-agent-control".to_string(),
            message: "delegate this".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut rx, "thread-agent-control").await;

    let teams = runtime.list_teams().await;
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].lead_thread_id, "thread-agent-control");
    assert_eq!(teams[0].members.len(), 2);
    assert_eq!(teams[0].members[1].name, "reviewer");
    assert_eq!(
        teams[0].members[1].status,
        roder_api::teams::TeamMemberStatus::Closed
    );
    assert_ne!(teams[0].members[1].thread_id, teams[0].lead_thread_id);
    let thread_ids = engine.thread_ids.lock().unwrap().clone();
    assert!(thread_ids.contains(&"thread-agent-control".to_string()));
    assert!(thread_ids.iter().any(|id| id != "thread-agent-control"));
    let tool_names_by_thread = engine.tool_names_by_thread.lock().unwrap().clone();
    let parent_tools = tool_names_by_thread
        .iter()
        .find(|(thread_id, _)| thread_id == "thread-agent-control")
        .map(|(_, tools)| tools)
        .unwrap();
    assert!(parent_tools.contains(&"spawn_agent".to_string()));
    assert!(parent_tools.contains(&"send_message".to_string()));
    assert!(parent_tools.contains(&"list_agents".to_string()));
    assert!(parent_tools.contains(&"wait_agent".to_string()));
    assert!(parent_tools.contains(&"close_agent".to_string()));
    let child_tools = tool_names_by_thread
        .iter()
        .find(|(thread_id, _)| thread_id != "thread-agent-control")
        .map(|(_, tools)| tools)
        .unwrap();
    assert!(child_tools.contains(&"spawn_agent".to_string()));
    assert!(child_tools.contains(&"send_message".to_string()));
    assert!(child_tools.contains(&"list_agents".to_string()));
    assert!(child_tools.contains(&"wait_agent".to_string()));
    assert!(child_tools.contains(&"close_agent".to_string()));
}

#[tokio::test]
async fn tool_execution_errors_are_returned_to_model() {
    let engine = Arc::new(ErrorRecoveringEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(FailingContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                auto_compact_token_limit: None,
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_tool_error".to_string(),
            message: "read missing file".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_tool_error").await;

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::ToolResult(result)
                if result.name.as_deref() == Some("failing_tool")
                    && result.is_error
                    && result.result == "path does not exist: crates/roder-tui/src/main.rs"
        )),
        "second request should include the tool error result: {:?}",
        requests[1].transcript
    );
}

#[tokio::test]
async fn run_turn_allows_more_than_eight_tool_rounds() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 9,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(EchoContributor));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: Some(10_000),
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_many_tools".to_string(),
            message: "keep using tools".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some("thread_many_tools")
        {
            break;
        }
    }

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 10);
}

#[tokio::test]
async fn unknown_tool_completion_is_marked_as_error() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 1,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: Some(10_000),
                file_backed_dynamic_context: true,
                hosted_web_search: roder_api::inference::HostedWebSearchConfig::disabled(),
                model_edit_tools: std::collections::HashMap::new(),
                model_parallel_tool_calls: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                tool_allowlist: Vec::new(),
                command_shell: roder_api::command_shell::default_command_shell(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
                runtime_profile: roder_api::inference::RuntimeProfile::Interactive,
                speed_policy: Default::default(),
                dynamic_workflows: Default::default(),
                reliability: Default::default(),
                turn_deadline_seconds: None,
                remote_runner_destination: None,
                team_data_dir: None,
                roadmap_data_dir: None,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_unknown_tool".to_string(),
            message: "echo please".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let roder_api::events::RoderEvent::ToolCallCompleted(completed) = event.event {
            assert!(completed.is_error);
            break;
        }
    }
}

#[test]
fn duplicate_tool_contributors_fail_with_contributor_context() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
        tool_rounds: 1,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.tool_contributor(Arc::new(DuplicateWebSearchContributor("search-a")));
    builder.tool_contributor(Arc::new(DuplicateWebSearchContributor("search-b")));

    let err = match builder.build() {
        Ok(_) => panic!("duplicate web_search tools should fail registry construction"),
        Err(err) => err,
    };
    let message = format!("{err:#}");

    assert!(
        message.contains("tool \"web_search\" is already registered"),
        "{message}"
    );
}

async fn wait_for_completed(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
) {
    wait_for_kind(events, thread_id, "turn.completed").await;
}

async fn wait_for_kind(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
    kind: &str,
) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == kind && event.thread_id.as_deref() == Some(thread_id) {
            break;
        }
    }
}

fn request_tool_names(request: &AgentInferenceRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}
