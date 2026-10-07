use super::*;
use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::goals::{ThreadGoalPatch, ThreadGoalStatus};
use roder_api::inference::{CompletionMetadata, InferenceEvent, MessageDelta, ToolCallCompleted};
use roder_api::policy_mode::PolicyMode;
use roder_core::RuntimeConfig;
use serde_json::json;

struct GoalWriteEngine {
    requests: std::sync::Mutex<Vec<AgentInferenceRequest>>,
    output: std::path::PathBuf,
}

#[async_trait]
impl InferenceEngine for GoalWriteEngine {
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
        let events = if first {
            vec![
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "goal-write".into(),
                    name: "write_file".into(),
                    arguments: json!({"path": self.output, "content": "Goal achieved"}).to_string(),
                }),
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "goal-complete".into(),
                    name: "update_goal".into(),
                    arguments: json!({"status": "complete"}).to_string(),
                }),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".into()),
                    provider_response_id: None,
                }),
            ]
        } else {
            vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "Done".into(),
                    phase: None,
                }),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("stop".into()),
                    provider_response_id: None,
                }),
            ]
        };
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

struct OrdinaryReview;
#[async_trait]
impl PolicyContributor for OrdinaryReview {
    fn id(&self) -> String {
        "ordinary-review".into()
    }
    async fn review_tool(&self, _: PolicyReview) -> anyhow::Result<PolicyContribution> {
        Ok(PolicyContribution::RequireApproval {
            reason: Some("ordinary extension review".into()),
        })
    }
}

#[tokio::test]
async fn acp_full_access_goal_executes_outside_workspace_without_permission_prompt() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let output = root.path().join("outside-workspace.txt");
    let engine = Arc::new(GoalWriteEngine {
        requests: std::sync::Mutex::new(vec![]),
        output: output.clone(),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.tool_contributor(
        roder_tools::builtin_coding_tools_contributor(workspace.clone()).unwrap(),
    );
    registry.policy_contributor(Arc::new(OrdinaryReview));
    let runtime = Arc::new(
        Runtime::new(
            registry.build().unwrap(),
            RuntimeConfig {
                policy_mode: PolicyMode::Bypass,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let server = Arc::new(AppServer::with_feature_config(
        runtime.clone(),
        AppServerFeatureConfig::default()
            .with_workspace_registry_path(root.path().join("registry.json")),
    ));
    let client = LocalAppClient::new(server);
    let adapter = AcpAdapter::new(client.clone());
    let peer = RecordingPeer::default();
    let session = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!("new")),
                method: "session/new".into(),
                params: Some(serde_json::to_value(acp::NewSessionRequest::new(workspace)).unwrap()),
            },
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    let session_id = session.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    // Native transport owns goal management; ACP continues to use its existing surface.
    let goal = client.send_request(JsonRpcRequest { jsonrpc: "2.0".into(), id: Some(json!("goal")),
        method: "thread/goal/set".into(), params: Some(json!({"threadId": session_id, "objective": "Write the requested output", "status": "paused"})) }).await;
    assert!(goal.error.is_none(), "{goal:?}");
    runtime
        .thread_goal_set(
            &session_id,
            ThreadGoalPatch {
                status: Some(ThreadGoalStatus::Active),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        adapter.handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!("prompt")),
                method: "session/prompt".into(),
                params: Some(
                    serde_json::to_value(acp::PromptRequest::new(
                        session_id.clone(),
                        vec![acp::ContentBlock::Text(acp::TextContent::new(
                            "Complete the goal",
                        ))],
                    ))
                    .unwrap(),
                ),
            },
            &peer,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
    assert_eq!(*peer.permission_requests.lock().await, 0);
    assert_eq!(std::fs::read_to_string(output).unwrap(), "Goal achieved");
    assert_eq!(
        runtime
            .thread_goal_get(&session_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ThreadGoalStatus::Complete
    );
    assert!(
        engine.requests.lock().unwrap()[0]
            .instructions
            .developer
            .as_ref()
            .unwrap()
            .contains("Goal permissions: Full Access")
    );
    let notifications = peer.notifications.lock().await;
    for kind in ["tool_call", "tool_call_update"] {
        assert!(
            notifications
                .iter()
                .any(|notification| notification.method == "session/update"
                    && notification.params["update"]["sessionUpdate"] == kind),
            "{notifications:?}"
        );
    }
}
