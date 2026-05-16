use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::conversation::ConversationItem;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;

struct ToolLoopEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
    tool_rounds: usize,
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

struct ErrorRecoveringEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

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
        let has_tool_error = requests.last().unwrap().conversation.iter().any(|item| {
            matches!(
                item,
                ConversationItem::ToolResult(result)
                    if result.is_error
                        && result.result.contains("path does not exist")
            )
        });
        drop(requests);

        let events = if has_tool_error {
            vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "recovered".to_string(),
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
                default_model: "mock".to_string(),
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: Some(10_000),
                model_edit_tools: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_1".to_string(),
            message: "echo please".to_string(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
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
            .is_some_and(|text| text.contains("You are Roder")),
        "first request should include the Roder system prompt"
    );
    assert!(
        requests[1]
            .conversation
            .iter()
            .any(|item| matches!(item, ConversationItem::ToolResult(result) if result.result == "from tool")),
        "second request should include the tool result: {:?}",
        requests[1].conversation
    );
}

#[tokio::test]
async fn runtime_advertises_apply_patch_only_for_patch_models() {
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
                model_edit_tools: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_patch_tools".to_string(),
            message: "patch please".to_string(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
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
                model_edit_tools: std::collections::HashMap::from([(
                    "custom-model".to_string(),
                    "patch".to_string(),
                )]),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
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
                model_edit_tools: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_tool_error".to_string(),
            message: "read missing file".to_string(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_tool_error").await;

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].conversation.iter().any(|item| matches!(
            item,
            ConversationItem::ToolResult(result)
                if result.name.as_deref() == Some("failing_tool")
                    && result.is_error
                    && result.result == "path does not exist: crates/roder-tui/src/main.rs"
        )),
        "second request should include the tool error result: {:?}",
        requests[1].conversation
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
                model_edit_tools: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_many_tools".to_string(),
            message: "keep using tools".to_string(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
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
                model_edit_tools: std::collections::HashMap::new(),
                workspace: None,
                policy_mode: roder_api::policy_mode::PolicyMode::Default,
            },
        )
        .unwrap(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_unknown_tool".to_string(),
            message: "echo please".to_string(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
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

    let err = match Runtime::new(builder.build().unwrap(), RuntimeConfig::default()) {
        Ok(_) => panic!("duplicate web_search tools should fail runtime construction"),
        Err(err) => err,
    };
    let message = format!("{err:#}");

    assert!(
        message.contains("tool contributor search-b failed"),
        "{message}"
    );
    assert!(
        message.contains("tool \"web_search\" is already registered"),
        "{message}"
    );
}

async fn wait_for_completed(
    events: &mut tokio::sync::broadcast::Receiver<roder_api::events::EventEnvelope>,
    thread_id: &str,
) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some(thread_id) {
            break;
        }
    }
}

fn request_tool_names(request: &AgentInferenceRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}
