use super::*;

use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEvent,
    InferenceEventStream, InferenceFailure, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;

struct PendingInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for PendingInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
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
        Ok(Box::pin(stream::pending()))
    }
}

struct PartialFailureInferenceEngine;

#[async_trait::async_trait]
impl InferenceEngine for PartialFailureInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
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
        Ok(Box::pin(stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "partial teammate result".to_string(),
                phase: None,
            })),
            Ok(InferenceEvent::Failed(InferenceFailure {
                message: "provider disconnected".to_string(),
            })),
        ])))
    }
}

#[tokio::test]
async fn interrupt_agent_is_a_noop_without_an_active_turn() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        lifecycle_runtime("interrupt-idle", Arc::new(FakeInferenceEngine)).await;
    let team = spawn_idle_member(&runtime, &parent_thread_id, "parent-turn", "reviewer").await;
    let member = team.members.last().unwrap().clone();
    let mut events = runtime.subscribe_events();

    let idle = execute_control_tool(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "interrupt_agent",
        serde_json::json!({ "target": member.thread_id.clone() }),
    )
    .await;
    assert!(!idle.is_error, "{}", idle.text);
    assert_eq!(idle.data["previous_status"], "idle");
    assert!(idle.data["interrupted_turn_id"].is_null());
    assert_eq!(
        runtime.read_team(&team.id).await.unwrap().members[1].status,
        TeamMemberStatus::Idle
    );

    runtime
        .teams
        .update_member(&team.id, &member.id, |member| {
            member.status = TeamMemberStatus::Completed;
            member.final_message = Some("already finished".to_string());
        })
        .await
        .unwrap();
    let completed = execute_control_tool(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "interrupt_agent",
        serde_json::json!({ "target": member.thread_id.clone() }),
    )
    .await;
    assert!(!completed.is_error, "{}", completed.text);
    assert_eq!(completed.data["previous_status"], "completed");
    assert!(completed.data["interrupted_turn_id"].is_null());
    let unchanged = runtime.read_team(&team.id).await.unwrap();
    assert_eq!(unchanged.members[1].status, TeamMemberStatus::Completed);
    assert_eq!(
        unchanged.members[1].final_message.as_deref(),
        Some("already finished")
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), events.recv())
            .await
            .is_err(),
        "an inactive interrupt must not emit another completion"
    );

    cleanup(thread_root, team_root);
}

#[tokio::test]
async fn wait_agent_returns_stored_terminal_result_details() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        lifecycle_runtime("wait-terminal", Arc::new(FakeInferenceEngine)).await;
    let team = spawn_idle_member(&runtime, &parent_thread_id, "parent-turn", "reviewer").await;
    let member = team.members.last().unwrap().clone();
    runtime
        .teams
        .update_member(&team.id, &member.id, |member| {
            member.status = TeamMemberStatus::Failed;
            member.final_message = Some("partial review".to_string());
            member.terminal_error = Some("provider failed".to_string());
        })
        .await
        .unwrap();

    let result = execute_control_tool(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "wait_agent",
        serde_json::json!({ "target": member.thread_id.clone(), "timeout_ms": 10_000 }),
    )
    .await;

    assert!(!result.is_error, "{}", result.text);
    assert_eq!(result.data["timed_out"], false);
    assert_eq!(result.data["agents"][0]["status"], "failed");
    assert_eq!(result.data["agents"][0]["final_message"], "partial review");
    assert_eq!(
        result.data["agents"][0]["terminal_error"],
        "provider failed"
    );

    cleanup(thread_root, team_root);
}

#[tokio::test]
async fn targetless_wait_prioritizes_live_agents_over_stale_terminal_results() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        lifecycle_runtime("wait-live-over-stale", Arc::new(FakeInferenceEngine)).await;
    let first = spawn_idle_member(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "finished_reviewer",
    )
    .await;
    let finished = first.members.last().unwrap().clone();
    let second = spawn_idle_member(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "active_reviewer",
    )
    .await;
    let active = second.members.last().unwrap().clone();
    runtime
        .teams
        .update_member(&second.id, &finished.id, |member| {
            member.status = TeamMemberStatus::Completed;
            member.final_message = Some("old result".to_string());
        })
        .await
        .unwrap();

    let mut wait = Box::pin(execute_control_tool(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "wait_agent",
        serde_json::json!({ "timeout_ms": 10_000 }),
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), wait.as_mut())
            .await
            .is_err(),
        "targetless wait must not return a stale terminal result while another agent is live"
    );

    runtime
        .emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
            team_id: second.id.clone(),
            member_id: active.id.clone(),
            member_thread_id: active.thread_id.clone(),
            turn_id: Some("active-turn".to_string()),
            status: TeamMemberStatus::Completed,
            final_message: Some("new result".to_string()),
            error: None,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), wait)
        .await
        .expect("live agent completion should wake targetless wait");
    assert!(!result.is_error, "{}", result.text);
    assert_eq!(result.data["agents"][0]["member_id"], active.id);
    assert_eq!(result.data["agents"][0]["final_message"], "new result");

    let explicit = execute_control_tool(
        &runtime,
        &parent_thread_id,
        "parent-turn",
        "wait_agent",
        serde_json::json!({
            "target": finished.thread_id,
            "timeout_ms": 10_000
        }),
    )
    .await;
    assert!(!explicit.is_error, "{}", explicit.text);
    assert_eq!(explicit.data["timed_out"], false);
    assert_eq!(explicit.data["agents"][0]["member_id"], finished.id);
    assert_eq!(explicit.data["agents"][0]["final_message"], "old result");

    cleanup(thread_root, team_root);
}

#[tokio::test]
async fn wait_agent_observes_mailbox_activity_queued_before_subscription() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        lifecycle_runtime("wait-pending-steer", Arc::new(PendingInferenceEngine)).await;
    let parent_turn_id = runtime
        .start_turn(StartTurnRequest {
            thread_id: parent_thread_id.clone(),
            message: "coordinate the review".to_string(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: std::env::current_dir().unwrap().display().to_string(),
            instructions: crate::default_instructions(),
            developer_context: None,
            task_ledger_required: false,
        })
        .await
        .unwrap();
    let team = spawn_idle_member(&runtime, &parent_thread_id, &parent_turn_id, "reviewer").await;
    let member = team.members.last().unwrap().clone();
    runtime
        .queue_team_member_message(
            &member.thread_id,
            &team.id,
            &team.members[0].id,
            "agent update".to_string(),
        )
        .await
        .unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        execute_control_tool(
            &runtime,
            &parent_thread_id,
            &parent_turn_id,
            "wait_agent",
            serde_json::json!({ "target": member.thread_id.clone(), "timeout_ms": 10_000 }),
        ),
    )
    .await
    .expect("pending mailbox activity should wake wait_agent without waiting for its timeout");

    assert!(!result.is_error, "{}", result.text);
    assert_eq!(result.data["timed_out"], false);
    assert_eq!(result.data["activity"], "mailbox_or_steer");
    runtime
        .interrupt_turn(parent_thread_id, parent_turn_id)
        .await
        .unwrap();
    cleanup(thread_root, team_root);
}

#[tokio::test]
async fn failed_agent_preserves_partial_final_message_and_error() {
    let (runtime, parent_thread_id, thread_root, team_root) =
        lifecycle_runtime("partial-failure", Arc::new(PartialFailureInferenceEngine)).await;
    let team = spawn_idle_member(&runtime, &parent_thread_id, "parent-turn", "reviewer").await;
    let member = team.members.last().unwrap().clone();
    let mut events = runtime.subscribe_events();

    runtime
        .followup_team_member(
            &parent_thread_id,
            &team.id,
            &member.id,
            "review the patch".to_string(),
        )
        .await
        .unwrap();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if let RoderEvent::TeamMemberCompleted(completed) = envelope.event
                && completed.member_id == member.id
            {
                break completed;
            }
        }
    })
    .await
    .expect("failed teammate should publish a terminal result");

    assert_eq!(completed.status, TeamMemberStatus::Failed);
    assert_eq!(
        completed.final_message.as_deref(),
        Some("partial teammate result")
    );
    assert_eq!(completed.error.as_deref(), Some("provider disconnected"));
    let stored = runtime.read_team(&team.id).await.unwrap();
    assert_eq!(
        stored.members[1].final_message.as_deref(),
        Some("partial teammate result")
    );
    assert_eq!(
        stored.members[1].terminal_error.as_deref(),
        Some("provider disconnected")
    );
    let terminal = stored
        .mailbox
        .iter()
        .find(|message| message.kind == TeamMailboxMessageKind::FinalAnswer)
        .expect("terminal result mailbox message");
    assert_eq!(terminal.from_member_id.as_deref(), Some(member.id.as_str()));
    assert_eq!(terminal.to_member_id, stored.members[0].id);
    assert!(terminal.text.contains("partial teammate result"));
    assert!(terminal.text.contains("provider disconnected"));

    cleanup(thread_root, team_root);
}

async fn execute_control_tool(
    runtime: &Arc<Runtime>,
    parent_thread_id: &ThreadId,
    parent_turn_id: &str,
    name: &str,
    arguments: serde_json::Value,
) -> roder_api::tools::ToolResult {
    runtime
        .execute_agent_control_tool(
            parent_thread_id,
            &parent_turn_id.to_string(),
            &ToolCallCompleted {
                id: format!("call-{name}"),
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
            arguments,
        )
        .await
}

async fn spawn_idle_member(
    runtime: &Arc<Runtime>,
    parent_thread_id: &ThreadId,
    parent_turn_id: &str,
    task_name: &str,
) -> TeamState {
    runtime
        .spawn_team_member_for_caller(
            parent_thread_id,
            &parent_turn_id.to_string(),
            TeamMemberStartRequest {
                name: task_name.to_string(),
                model_provider: None,
                model: None,
            },
            None,
            "none".to_string(),
            None,
        )
        .await
        .unwrap()
}

async fn lifecycle_runtime(
    test_name: &str,
    engine: Arc<dyn InferenceEngine>,
) -> (Arc<Runtime>, ThreadId, PathBuf, PathBuf) {
    let suffix = uuid::Uuid::new_v4();
    let thread_root =
        std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-threads-{suffix}"));
    let team_root = std::env::temp_dir().join(format!("roder-codex-v2-{test_name}-teams-{suffix}"));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine);
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: thread_root.clone(),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.to_string(),
                default_model: "mock".to_string(),
                reasoning: None,
                team_data_dir: Some(team_root.clone()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let parent = runtime
        .create_thread_with(CreateThreadRequest {
            title: Some("Codex V2 lifecycle parent".to_string()),
            workspace: std::env::current_dir().unwrap().display().to_string(),
            workspace_id: Some("workspace-test".to_string()),
            root_id: Some("root-test".to_string()),
            provider: Some(PROVIDER_MOCK.to_string()),
            model: Some("mock".to_string()),
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner: None,
        })
        .await
        .unwrap();
    (runtime, parent.thread_id, thread_root, team_root)
}

fn cleanup(thread_root: PathBuf, team_root: PathBuf) {
    let _ = std::fs::remove_dir_all(thread_root);
    let _ = std::fs::remove_dir_all(team_root);
}
