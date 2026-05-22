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

struct HugeOutputEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for HugeOutputEngine {
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
                    id: "call_huge_output".to_string(),
                    name: "huge_output".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: Some("resp_tool".to_string()),
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

struct HugeOutputContributor;

impl ToolContributor for HugeOutputContributor {
    fn id(&self) -> ToolProviderId {
        "test-huge-output".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(HugeOutputTool))
    }
}

struct HugeOutputTool;

#[async_trait::async_trait]
impl ToolExecutor for HugeOutputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "huge_output".to_string(),
            description: "Return a single huge line".to_string(),
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
            text: format!("start{}end", "x".repeat(100_000)),
            data: json!({}),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn oversized_tool_result_is_capped_before_next_provider_request() {
    let engine = Arc::new(HugeOutputEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(HugeOutputContributor));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_huge_tool_output".to_string(),
            message: "produce huge tool output".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: None,

            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    wait_for_completed(&mut events, "thread_huge_tool_output").await;

    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let tool_result = requests[1]
        .conversation
        .iter()
        .find_map(|item| match item {
            ConversationItem::ToolResult(result) => Some(result),
            _ => None,
        })
        .expect("second provider request should include tool result");

    assert_eq!(tool_result.name.as_deref(), Some("huge_output"));
    assert!(tool_result.result.chars().count() <= 20_000);
    assert!(tool_result.result.starts_with(
        "Tool output was stored in a local context artifact because it exceeded inline limits."
    ));
    assert!(tool_result.result.contains("[artifact: tool_output"));
    assert!(tool_result.result.contains("read_artifact"));
    assert!(tool_result.result.ends_with("end"));
    assert!(tool_result.result.contains("chars omitted"));
    assert!(!tool_result.result.contains(&"x".repeat(50_000)));
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
