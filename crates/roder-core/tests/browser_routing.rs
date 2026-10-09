//! The browser routing block reaches the model only when the turn advertises
//! two or more browser tool families (report item 11).
//!
//! A one-answer engine records the request it is sent; fake tools stand in
//! for the extensions, since only the tool names matter.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;

const HEADING: &str = "## Browser Tool Routing";

struct RecordingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for RecordingEngine {
    fn id(&self) -> InferenceEngineId {
        "mock-routing".to_string()
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
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter(vec![
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

struct NamedTools(&'static [&'static str]);

impl ToolContributor for NamedTools {
    fn id(&self) -> ToolProviderId {
        "test-browser-families".to_string()
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        for name in self.0 {
            registry.register(Arc::new(NamedTool { name }))?;
        }
        Ok(())
    }
}

struct NamedTool {
    name: &'static str,
}

#[async_trait::async_trait]
impl ToolExecutor for NamedTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: "Fake browser tool".to_string(),
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
            text: "ok".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}

/// The request the engine got for one turn on a runtime that has `tools`.
async fn request_with(tools: &'static [&'static str]) -> AgentInferenceRequest {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(RecordingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(NamedTools(tools)));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread_browser_routing".to_string(),
            message: "look at the page".to_string(),
            images: Vec::new(),
            provider_override: Some("mock-routing".to_string()),
            model_override: Some("scripted-model".to_string()),
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), events.recv())
            .await
            .expect("turn did not finish")
            .unwrap();
        assert_ne!(event.kind, "turn.failed", "{event:?}");
        if event.kind == "turn.completed" {
            break;
        }
    }
    let mut requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "one round");
    requests.remove(0)
}

fn developer(request: &AgentInferenceRequest) -> String {
    request.instructions.developer.clone().unwrap_or_default()
}

#[tokio::test]
async fn two_families_get_the_routing_block_with_only_their_lines() {
    let request = request_with(&["jev_browse", "chrome_click", "chrome_navigate"]).await;
    let developer = developer(&request);
    assert_eq!(developer.matches(HEADING).count(), 1, "{developer}");
    assert!(developer.contains("- `jev_browse`"), "{developer}");
    assert!(developer.contains("- `chrome_*`"), "{developer}");
    assert!(!developer.contains("browser_use_"), "{developer}");
    assert!(!developer.contains("webwright"), "{developer}");
}

#[tokio::test]
async fn one_family_gets_no_routing_block() {
    let request = request_with(&["chrome_click", "chrome_navigate", "chrome_eval"]).await;
    let developer = developer(&request);
    assert!(!developer.contains(HEADING), "{developer}");
}

#[tokio::test]
async fn the_block_is_identical_for_the_same_tool_set_on_every_turn() {
    let tools = &["browser_use_click", "webwright.run_script", "chrome_click"];
    let first = developer(&request_with(tools).await);
    let second = developer(&request_with(tools).await);
    assert!(first.contains(HEADING), "{first}");
    assert_eq!(first, second);
}
