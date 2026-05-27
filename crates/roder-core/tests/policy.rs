use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::context::{PolicyContribution, PolicyContributor, PolicyGate, PolicyReview};
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
fn plan_mode_allows_read_like_tool_with_write_like_arguments() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call(
            "read_metadata",
            json!({ "operation": "fs.write", "path": "src/lib.rs" }),
        ),
        PolicyMode::Plan,
        &context(PolicyMode::Plan),
    );

    assert!(matches!(decision, PolicyDecision::Allowed));
}

#[test]
fn grep_query_containing_destructive_words_is_allowed() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call(
            "grep",
            json!({ "query": "edit command patch", "path": "." }),
        ),
        PolicyMode::Default,
        &context(PolicyMode::Default),
    );

    assert!(matches!(decision, PolicyDecision::Allowed));
}

#[test]
fn plan_mode_denies_write_tool_name() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("fs.write", json!({ "path": "src/lib.rs" })),
        PolicyMode::Plan,
        &context(PolicyMode::Plan),
    );

    assert!(matches!(decision, PolicyDecision::Denied { .. }));
}

#[test]
fn plan_mode_denies_shell_tool_name() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("shell", json!({ "command": "cargo test" })),
        PolicyMode::Plan,
        &context(PolicyMode::Plan),
    );

    assert!(matches!(decision, PolicyDecision::Denied { .. }));
}

#[test]
fn default_mode_shell_still_requires_approval() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("shell", json!({ "command": "cargo test" })),
        PolicyMode::Default,
        &context(PolicyMode::Default),
    );

    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
}

#[test]
fn default_mode_edit_still_requires_approval() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("fs.edit", json!({ "path": "src/lib.rs" })),
        PolicyMode::Default,
        &context(PolicyMode::Default),
    );

    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
}

#[test]
fn accept_all_auto_approves_process_spawn() {
    let decision = DefaultPolicyGate::new().decide(
        &tool_call("process.spawn", json!({ "cmd": "cargo test" })),
        PolicyMode::AcceptAll,
        &context(PolicyMode::AcceptAll),
    );

    assert!(matches!(decision, PolicyDecision::AutoApproved { .. }));
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
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
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
async fn policy_plan_mode_denies_write_tool_and_skips_executor() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Plan,
        "fs.write",
        json!({ "path": "src/lib.rs" }),
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
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),

            instructions: default_instructions(),
            task_ledger_required: false,
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

#[tokio::test]
async fn policy_default_mode_grep_executes_without_approval_for_destructive_query_text() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Default,
        "grep",
        json!({ "query": "edit command patch", "path": "." }),
        seen_modes.clone(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-grep".to_string(),
            message: "search files".to_string(),
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

    let mut saw_approval = false;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            RoderEvent::ApprovalRequested(_) => saw_approval = true,
            RoderEvent::TurnCompleted(_) => break,
            _ => {}
        }
    }

    assert!(!saw_approval);
    assert_eq!(*seen_modes.lock().unwrap(), vec![PolicyMode::Default]);
}

#[tokio::test]
async fn policy_default_mode_process_waits_for_approval_before_executing() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Default,
        "process.spawn",
        json!({ "cmd": "cargo test" }),
        seen_modes.clone(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-approval".to_string(),
            message: "run command".to_string(),
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

    let approval_id = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ApprovalRequested(event) = envelope.event {
            assert_eq!(event.tool_name, "process.spawn");
            break event.approval_id;
        }
    };

    assert!(seen_modes.lock().unwrap().is_empty());

    runtime
        .resolve_tool_approval(&approval_id, true)
        .await
        .unwrap();

    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(envelope.event, RoderEvent::TurnCompleted(_)) {
            break;
        }
    }

    assert_eq!(*seen_modes.lock().unwrap(), vec![PolicyMode::Default]);
}

#[tokio::test]
async fn switching_to_accept_all_auto_approves_pending_shell_tool() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy(
        PolicyMode::Default,
        "shell",
        json!({ "command": "cargo test" }),
        seen_modes.clone(),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-live-approval".to_string(),
            message: "run tests".to_string(),
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

    let approval_id = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ApprovalRequested(event) = envelope.event {
            assert_eq!(event.tool_name, "shell");
            break event.approval_id;
        }
    };

    assert!(seen_modes.lock().unwrap().is_empty());
    runtime
        .set_policy_mode(PolicyMode::AcceptAll, Some("test mode switch".to_string()))
        .await
        .unwrap();

    let mut saw_auto_approval_resolution = false;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            RoderEvent::ApprovalResolved(event) => {
                saw_auto_approval_resolution = event.approval_id == approval_id && event.approved;
            }
            RoderEvent::TurnCompleted(_) => break,
            _ => {}
        }
    }

    assert!(saw_auto_approval_resolution);
    assert_eq!(*seen_modes.lock().unwrap(), vec![PolicyMode::AcceptAll]);
}

#[tokio::test]
async fn extension_policy_contributor_can_deny_tool_call() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy_contributor(
        PolicyMode::Default,
        "grep",
        json!({ "query": "needle" }),
        seen_modes.clone(),
        Arc::new(StaticPolicyContributor {
            id: "deny-grep",
            contribution: PolicyContribution::Deny {
                reason: "extension policy blocked grep".to_string(),
            },
        }),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-extension-policy".to_string(),
            message: "search files".to_string(),
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

    let mut saw_denied = false;
    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match envelope.event {
            RoderEvent::PolicyDecisionRecorded(event) => {
                saw_denied = matches!(
                    event.decision,
                    PolicyDecision::Denied { reason } if reason.contains("extension policy blocked grep")
                );
            }
            RoderEvent::TurnCompleted(_) => break,
            _ => {}
        }
    }

    assert!(saw_denied);
    assert!(seen_modes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn extension_policy_contributor_can_require_approval() {
    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime_with_policy_contributor(
        PolicyMode::Default,
        "grep",
        json!({ "query": "needle" }),
        seen_modes.clone(),
        Arc::new(StaticPolicyContributor {
            id: "approve-grep",
            contribution: PolicyContribution::RequireApproval {
                reason: Some("extension policy wants review".to_string()),
            },
        }),
    );
    let mut events = runtime.subscribe_events();

    runtime
        .start_turn(StartTurnRequest {
            thread_id: "thread-extension-approval".to_string(),
            message: "search files".to_string(),
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

    let approval_id = loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let RoderEvent::ApprovalRequested(event) = envelope.event {
            assert_eq!(event.tool_name, "grep");
            assert_eq!(
                event.reason.as_deref(),
                Some("extension policy wants review")
            );
            break event.approval_id;
        }
    };
    assert!(seen_modes.lock().unwrap().is_empty());

    runtime
        .resolve_tool_approval(&approval_id, true)
        .await
        .unwrap();

    loop {
        let envelope = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(envelope.event, RoderEvent::TurnCompleted(_)) {
            break;
        }
    }

    assert_eq!(*seen_modes.lock().unwrap(), vec![PolicyMode::Default]);
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

fn runtime_with_policy_contributor(
    policy_mode: PolicyMode,
    tool_name: &'static str,
    arguments: serde_json::Value,
    seen_modes: Arc<Mutex<Vec<PolicyMode>>>,
    contributor: Arc<dyn PolicyContributor>,
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
    builder.policy_contributor(contributor);
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
    ToolExecutionContext::new("thread", "turn", mode)
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

struct StaticPolicyContributor {
    id: &'static str,
    contribution: PolicyContribution,
}

#[async_trait::async_trait]
impl PolicyContributor for StaticPolicyContributor {
    fn id(&self) -> roder_api::extension::PolicyContributorId {
        self.id.to_string()
    }

    async fn review_tool(&self, _review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        Ok(self.contribution.clone())
    }
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
