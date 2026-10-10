//! The browser routing block reaches the model only when the turn advertises
//! two or more browser tool families (report item 11), and only on a round
//! whose request can still call them.
//!
//! A scripted engine records the requests it is sent; fake tools stand in for
//! the extensions, since only the tool names matter.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;

const HEADING: &str = "## Browser Tool Routing";
const PARALLEL_HEADING: &str = "## Parallel Web Access";
/// The one fake tool that takes long enough to push the next round into the
/// deadline reserve.
const SLOW_TOOL: &str = "slow_tool";

struct RecordingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
    /// When set, the first round answers with a call to this tool instead of
    /// text, so the turn runs a second round.
    first_round_tool_call: Option<&'static str>,
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
        let first_round = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len() == 1
        };
        if let Some(tool) = self.first_round_tool_call.filter(|_| first_round) {
            return Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call-1".to_string(),
                    name: tool.to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])));
        }
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
        if self.name == SLOW_TOOL {
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "ok".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}

/// How the turn under test is run.
#[derive(Default)]
struct Scenario {
    /// Eval profile with a 3 second deadline: a round that starts after the
    /// slow tool finished is the deadline finalization round, sent with no
    /// tools.
    eval_deadline: bool,
    /// Eval profile asking for a task ledger: the first round is sent with
    /// only the ledger tool and forces a call to it.
    task_ledger_required: bool,
    first_round_tool_call: Option<&'static str>,
}

/// Every request the engine got for one turn on a runtime that has `tools`.
async fn requests_for(
    tools: &'static [&'static str],
    scenario: Scenario,
) -> Vec<AgentInferenceRequest> {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(RecordingEngine {
        requests: Mutex::new(Vec::new()),
        first_round_tool_call: scenario.first_round_tool_call,
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(NamedTools(tools)));
    if scenario.task_ledger_required {
        builder.tool_contributor(Arc::new(
            roder_ext_task_ledger::TaskLedgerToolContributor::default(),
        ));
    }
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    let mut config = RuntimeConfig::default();
    if scenario.eval_deadline || scenario.task_ledger_required {
        config.runtime_profile = RuntimeProfile::Eval;
        config.policy_mode = PolicyMode::Bypass;
    }
    if scenario.eval_deadline {
        config.turn_deadline_seconds = Some(3);
    }
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
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
            task_ledger_required: scenario.task_ledger_required,
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
    engine.requests.lock().unwrap().clone()
}

/// The request the engine got for the one round of a plain turn.
async fn request_with(tools: &'static [&'static str]) -> AgentInferenceRequest {
    let mut requests = requests_for(tools, Scenario::default()).await;
    assert_eq!(requests.len(), 1, "one round");
    requests.remove(0)
}

fn developer(request: &AgentInferenceRequest) -> String {
    request.instructions.developer.clone().unwrap_or_default()
}

fn tool_names(request: &AgentInferenceRequest) -> Vec<&str> {
    request
        .tools
        .iter()
        .map(|spec| spec.name.as_str())
        .collect()
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

#[tokio::test]
async fn a_forced_task_ledger_round_gets_no_routing_block() {
    let requests = requests_for(
        &["jev_browse", "browser_use_navigate", "parallel_search"],
        Scenario {
            task_ledger_required: true,
            ..Scenario::default()
        },
    )
    .await;
    let first = &requests[0];
    // The round offers the ledger tool and nothing else, so the families and
    // web tools the blocks describe are not callable in it.
    assert_eq!(tool_names(first), ["task_ledger.update"]);
    let developer = developer(first);
    assert!(!developer.contains(HEADING), "{developer}");
    assert!(!developer.contains(PARALLEL_HEADING), "{developer}");
}

#[tokio::test]
async fn the_deadline_finalization_round_gets_no_routing_block() {
    let requests = requests_for(
        &[
            SLOW_TOOL,
            "jev_browse",
            "browser_use_navigate",
            "parallel_search",
        ],
        Scenario {
            eval_deadline: true,
            first_round_tool_call: Some(SLOW_TOOL),
            ..Scenario::default()
        },
    )
    .await;
    assert_eq!(requests.len(), 2, "one tool round, one finalization round");
    // The ordinary round offers the families, so it is told how to pick.
    assert!(tool_names(&requests[0]).contains(&"jev_browse"));
    let ordinary = developer(&requests[0]);
    assert!(ordinary.contains(HEADING), "{ordinary}");
    assert!(ordinary.contains(PARALLEL_HEADING), "{ordinary}");
    // The finalization round is sent with no tools, so neither block applies.
    assert!(requests[1].tools.is_empty());
    let finalization = developer(&requests[1]);
    assert!(!finalization.contains(HEADING), "{finalization}");
    assert!(!finalization.contains(PARALLEL_HEADING), "{finalization}");
}
