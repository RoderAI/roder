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
        let events = if turn == 1 {
            vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_1".to_string(),
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

#[tokio::test]
async fn run_turn_continues_after_tool_result() {
    let engine = Arc::new(ToolLoopEngine {
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
                reasoning: Some("low".to_string()),
                auto_compact_token_limit: Some(10_000),
                workspace: None,
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

#[test]
fn duplicate_tool_contributors_fail_with_contributor_context() {
    let engine = Arc::new(ToolLoopEngine {
        requests: Mutex::new(Vec::new()),
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

    assert!(message.contains("tool contributor search-b failed"), "{message}");
    assert!(
        message.contains("tool \"web_search\" is already registered"),
        "{message}"
    );
}
