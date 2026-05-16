use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::context::PolicyGate;
use roder_api::events::RoderEvent;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId, ToolProviderId};
use roder_api::inference::*;
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolResult, ToolSpec,
};
use roder_core::policy_gate::DefaultPolicyGate;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;

#[test]
fn plan_mode_denies_write_like_arguments_even_with_benign_tool_name() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call(
            "read_metadata",
            json!({ "operation": "fs.write", "path": "src/lib.rs" }),
        ),
        PolicyMode::Plan,
        &context(PolicyMode::Plan),
    );

    assert!(matches!(decision, PolicyDecision::Denied { .. }));
}

#[test]
fn accept_edits_does_not_auto_approve_process_spawn() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("process.spawn", json!({ "cmd": "cargo test" })),
        PolicyMode::AcceptEdits,
        &context(PolicyMode::AcceptEdits),
    );

    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
}

#[tokio::test]
async fn policy_bypass_mode_emits_bypass_active_and_executes_with_effective_mode() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Bypass,
        "process.spawn",
        json!({ "cmd": "cargo test" }),
        seen_modes.clone(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-policy".to_string(),
            message: "run command".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
        })
        .await
        .unwrap();

    let mut saw_auto_approved = false;
    let mut saw_bypass_active = false;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            RoderEvent::PolicyDecisionRecorded(event) => {
                saw_auto_approved = matches!(
                    event.decision,
                    PolicyDecision::AutoApproved { matched_rule: Some(rule) } if rule == "*"
                );
            }
            RoderEvent::PolicyBypassActive(_) => saw_bypass_active = true,
            RoderEvent::TurnCompleted(_) => break,
            _ => {}
        }
    }

    assert!(saw_auto_approved);
    assert!(saw_bypass_active);
    assert_eq!(*seen_modes.lock().unwrap(), vec![PolicyMode::Bypass]);
}

#[tokio::test]
async fn policy_plan_mode_denial_skips_tool_executor() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Plan,
        "read_metadata",
        json!({ "requested_action": "fs.write", "path": "src/lib.rs" }),
        seen_modes.clone(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-plan".to_string(),
            message: "inspect only".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            instructions: default_instructions(),
        })
        .await
        .unwrap();

    let mut saw_denied = false;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            RoderEvent::PolicyDecisionRecorded(event) => {
                saw_denied = matches!(event.decision, PolicyDecision::Denied { .. });
            }
            RoderEvent::TurnCompleted(_) => break,
            _ => {}
        }
    }

    assert!(saw_denied);
    assert!(seen_modes.lock().unwrap().is_empty());
}

fn runtime_with_policy(
    policy_mode: PolicyMode,
    tool_name: &'static str,
    arguments: serde_json::Value,
    seen_modes: Arc<Mutex<Vec<PolicyMode>>>,
) -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(SingleToolCallEngine {
        tool_name,
        arguments,
        requests: Mutex::new(0),
    }));
    builder.tool_contributor(Arc::new(RecordingContributor {
        tool_name,
        seen_modes,
    }));
    Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                policy_mode,
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    )
}

fn tool_call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call-policy".to_string(),
        name: name.to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
    }
}

fn context(mode: PolicyMode) -> ToolExecutionContext {
    ToolExecutionContext {
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
        effective_mode: mode,
    }
}

struct SingleToolCallEngine {
    tool_name: &'static str,
    arguments: serde_json::Value,
    requests: Mutex<usize>,
}

#[async_trait::async_trait]
impl InferenceEngine for SingleToolCallEngine {
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
        let mut requests = self.requests.lock().unwrap();
        *requests += 1;
        if *requests == 1 {
            Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call-policy".to_string(),
                    name: self.tool_name.to_string(),
                    arguments: self.arguments.to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".to_string()),
                    provider_response_id: None,
                })),
            ])))
        } else {
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
}

struct RecordingContributor {
    tool_name: &'static str,
    seen_modes: Arc<Mutex<Vec<PolicyMode>>>,
}

impl ToolContributor for RecordingContributor {
    fn id(&self) -> ToolProviderId {
        format!("test-{}", self.tool_name)
    }

    fn contribute(&self, registry: &mut roder_api::tools::ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(RecordingTool {
            name: self.tool_name.to_string(),
            seen_modes: self.seen_modes.clone(),
        }))
    }
}

struct RecordingTool {
    name: String,
    seen_modes: Arc<Mutex<Vec<PolicyMode>>>,
}

#[async_trait::async_trait]
impl ToolExecutor for RecordingTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: "Record the effective policy mode.".to_string(),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        self.seen_modes.lock().unwrap().push(ctx.effective_mode);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "ok".to_string(),
            data: json!({}),
            is_error: false,
        })
    }
}
