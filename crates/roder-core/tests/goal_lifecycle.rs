use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{StreamExt, stream};
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::goals::{ThreadGoalPatch, ThreadGoalStatus};
use roder_api::inference::*;
use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};
use roder_core::{Runtime, StartTurnRequest, default_instructions};

struct LifecycleEngine {
    failure: Option<ProviderFailureKind>,
    requests: Mutex<Vec<AgentInferenceRequest>>,
    complete_with_tool: bool,
}

#[async_trait::async_trait]
impl InferenceEngine for LifecycleEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![])
    }
    async fn stream_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let first = requests.len() == 1;
        drop(requests);
        let mut events = vec![Ok(InferenceEvent::Usage(TokenUsage::new(20, 10, 30)))];
        if let Some(kind) = self.failure {
            events.push(Err(
                ProviderFailure::new(kind, "terminal provider failure").into()
            ));
            return Ok(Box::pin(stream::iter(events)));
        }
        if self.complete_with_tool {
            if first {
                events.push(Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "complete".into(),
                    name: "update_goal".into(),
                    arguments: r#"{"status":"complete"}"#.into(),
                })));
            } else {
                events.push(Ok(InferenceEvent::MessageDelta(MessageDelta {
                    text: "Finished".into(),
                    phase: None,
                })));
            }
            events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some(if first { "tool_calls" } else { "stop" }.into()),
                provider_response_id: None,
            })));
            return Ok(Box::pin(stream::iter(events)));
        }
        Ok(Box::pin(stream::iter(events).chain(stream::pending())))
    }
}

fn setup(
    failure: Option<ProviderFailureKind>,
    complete_with_tool: bool,
) -> (Arc<Runtime>, Arc<LifecycleEngine>) {
    let engine = Arc::new(LifecycleEngine {
        failure,
        complete_with_tool,
        requests: Mutex::new(vec![]),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.tool_contributor(
        roder_tools::builtin_coding_tools_contributor(std::env::current_dir().unwrap()).unwrap(),
    );
    (
        Arc::new(Runtime::new(registry.build().unwrap(), Default::default()).unwrap()),
        engine,
    )
}

async fn start(runtime: &Arc<Runtime>, id: &str) -> String {
    runtime
        .thread_goal_set(
            &id.into(),
            ThreadGoalPatch {
                objective: Some("Verify lifecycle".into()),
                token_budget: Some(Some(100)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: id.into(),
            message: "Do goal work".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn interrupt_pauses_and_preserves_streamed_usage() {
    let (runtime, _) = setup(None, false);
    let mut events = runtime.subscribe_events();
    let id = "interrupt-goal".to_string();
    let turn = start(&runtime, &id).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime
            .thread_goal_get(&id)
            .await
            .unwrap()
            .unwrap()
            .tokens_used
            != 30
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    runtime.interrupt_turn(id.clone(), turn).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while events.recv().await.unwrap().kind != "turn.interrupted" {}
    })
    .await
    .unwrap();
    let goal = runtime.thread_goal_get(&id).await.unwrap().unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Paused);
    assert_eq!(goal.tokens_used, 30);
}

#[tokio::test]
async fn terminal_errors_stop_goals_with_accounted_usage() {
    for (kind, status) in [
        (
            ProviderFailureKind::Authentication,
            ThreadGoalStatus::Blocked,
        ),
        (
            ProviderFailureKind::UsageLimit,
            ThreadGoalStatus::UsageLimited,
        ),
    ] {
        let (runtime, _) = setup(Some(kind), false);
        let mut events = runtime.subscribe_events();
        let id = format!("failure-{kind:?}");
        start(&runtime, &id).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.recv().await.unwrap().kind != "turn.failed" {}
        })
        .await
        .unwrap();
        let goal = runtime.thread_goal_get(&id).await.unwrap().unwrap();
        assert_eq!(goal.status, status);
        assert_eq!(goal.tokens_used, 30);
    }
}

#[tokio::test]
async fn real_completion_tool_reports_current_usage_before_final_response() {
    let (runtime, engine) = setup(None, true);
    let mut events = runtime.subscribe_events();
    let id = "completion-goal".to_string();
    start(&runtime, &id).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while events.recv().await.unwrap().kind != "turn.completed" {}
    })
    .await
    .unwrap();
    let goal = runtime.thread_goal_get(&id).await.unwrap().unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Complete);
    assert_eq!(
        goal.tokens_used, 30,
        "wrap-up inference after completion must not charge the stopped goal"
    );
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let serialized = serde_json::to_string(&requests[1].transcript).unwrap();
    assert!(serialized.contains("completionBudgetReport"));
    assert!(serialized.contains("tokensUsed\\\":30"), "{serialized}");
}
