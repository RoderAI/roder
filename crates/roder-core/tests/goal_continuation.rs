use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::goals::{ThreadGoalPatch, ThreadGoalStatus};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor, ToolCallCompleted,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use roder_api::transcript::TranscriptItem;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;

struct GoalContinuationEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for GoalContinuationEngine {
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
        let request_number = requests.len();
        drop(requests);

        let events = match request_number {
            1 => vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "leaving goal active for continuation".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ],
            2 => vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_get_goal".to_string(),
                    name: "get_goal".to_string(),
                    arguments: "{}".to_string(),
                })),
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call_complete_goal".to_string(),
                    name: "update_goal".to_string(),
                    arguments: json!({ "status": "complete" }).to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ],
            _ => vec![
                Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "goal complete".to_string(),
                    phase: None,
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".to_string()),
                    provider_response_id: None,
                })),
            ],
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

struct GoalToolsContributor;

impl ToolContributor for GoalToolsContributor {
    fn id(&self) -> ToolProviderId {
        "test-goal-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(TestGoalTool {
            name: "get_goal",
            description: "Read the active test goal.",
            parameters: json!({ "type": "object", "properties": {} }),
        }))?;
        registry.register(Arc::new(TestGoalTool {
            name: "update_goal",
            description: "Update the active test goal.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["complete", "blocked"]
                    }
                },
                "required": ["status"]
            }),
        }))
    }
}

struct TestGoalTool {
    name: &'static str,
    description: &'static str,
    parameters: serde_json::Value,
}

#[async_trait::async_trait]
impl ToolExecutor for TestGoalTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: self.parameters.clone(),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let controller = ctx.require_goal_controller()?;
        let goal = match call.name.as_str() {
            "get_goal" => controller.get_thread_goal(&call.thread_id).await?,
            "update_goal" => {
                let status = match call
                    .arguments
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("complete") => ThreadGoalStatus::Complete,
                    Some("blocked") => ThreadGoalStatus::Blocked,
                    other => anyhow::bail!("unsupported goal status: {other:?}"),
                };
                controller
                    .set_thread_goal(
                        &call.thread_id,
                        ThreadGoalPatch {
                            status: Some(status),
                            ..ThreadGoalPatch::default()
                        },
                    )
                    .await?
            }
            other => anyhow::bail!("unknown goal tool: {other}"),
        };
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: goal
                .as_ref()
                .map(|goal| goal.status.as_str().to_string())
                .unwrap_or_else(|| "no goal".to_string()),
            data: json!({ "goal": goal }),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn active_goal_continues_after_turn_until_model_completes_goal() {
    let engine = Arc::new(GoalContinuationEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(GoalToolsContributor));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();
    let thread_id = "thread_goal_continuation".to_string();

    runtime
        .thread_goal_set(
            &thread_id,
            ThreadGoalPatch {
                objective: Some("prove autonomous goal continuation".to_string()),
                status: Some(ThreadGoalStatus::Active),
                token_budget: None,
            },
        )
        .await
        .unwrap();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: thread_id.clone(),
            message: "start goal work".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            workspace: None,
            instructions: default_instructions(),
            task_ledger_required: false,
        })
        .await
        .unwrap();

    let mut completed_turns = 0;
    while completed_turns < 2 {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if event.kind == "turn.completed" && event.thread_id.as_deref() == Some(&thread_id) {
            completed_turns += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let goal = runtime
        .thread_goal_get(&thread_id)
        .await
        .unwrap()
        .expect("goal should still be present");
    assert_eq!(goal.status, ThreadGoalStatus::Complete);

    let requests = engine.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "expected initial turn, continuation tool round, and continuation final round"
    );
    assert!(
        requests[0]
            .instructions
            .developer
            .as_deref()
            .is_some_and(|text| text.contains("prove autonomous goal continuation")),
        "initial request should include active-goal instructions"
    );
    assert!(
        requests[1].transcript.iter().any(|item| matches!(
            item,
            TranscriptItem::UserMessage(message)
                if message.text.contains("Continue working autonomously toward the active goal")
        )),
        "continuation request should be started by the runtime: {:?}",
        requests[1].transcript
    );
    let continuation_tools = requests[1]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(continuation_tools.contains(&"get_goal"));
    assert!(continuation_tools.contains(&"update_goal"));
}
