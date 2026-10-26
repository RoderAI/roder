use super::*;
use roder_api::transcript::TranscriptItem;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use std::path::Path;

struct HistoryEngine(Arc<Mutex<Vec<AgentInferenceRequest>>>);

#[async_trait]
impl InferenceEngine for HistoryEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }
    async fn list_models(
        &self,
        ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        FakeInferenceEngine.list_models(ctx).await
    }
    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.0.lock().await.push(request.clone());
        if request.transcript.iter().any(|item| {
            matches!(item,
            TranscriptItem::UserMessage(message) if message.text == "ask permission")
        }) {
            use roder_api::inference::{CompletionMetadata, InferenceEvent, ToolCallCompleted};
            return Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "permission-tool".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({"path":"denied.txt","content":"must not run"})
                        .to_string(),
                })),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_calls".into()),
                    provider_response_id: None,
                })),
            ])));
        }
        FakeInferenceEngine.stream_turn(ctx, request).await
    }
}

fn durable_adapter(
    root: &Path,
    requests: Arc<Mutex<Vec<AgentInferenceRequest>>>,
) -> AcpAdapter<LocalAppClient> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(HistoryEngine(requests)));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: root.join("threads"),
    }));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let config = AppServerFeatureConfig::default()
        .with_workspace_registry_path(root.join("workspaces.json"));
    AcpAdapter::new(LocalAppClient::new(Arc::new(
        AppServer::with_feature_config(runtime, config),
    )))
}

async fn rpc<P: AcpClientPeer>(
    adapter: &AcpAdapter<LocalAppClient>,
    peer: &P,
    method: &str,
    params: serde_json::Value,
) -> roder_protocol::JsonRpcResponse {
    tokio::time::timeout(
        Duration::from_secs(30),
        adapter.handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(serde_json::json!(method)),
                method: method.into(),
                params: Some(params),
            },
            peer,
        ),
    )
    .await
    .expect("ACP operation timed out")
    .unwrap()
    .unwrap()
}

fn reopen(id: &str, cwd: &Path) -> serde_json::Value {
    serde_json::json!({"sessionId": id, "cwd": cwd, "mcpServers": []})
}

async fn prompt<P: AcpClientPeer>(
    adapter: &AcpAdapter<LocalAppClient>,
    peer: &P,
    id: &str,
    text: &str,
) -> roder_protocol::JsonRpcResponse {
    rpc(
        adapter,
        peer,
        "session/prompt",
        serde_json::json!({"sessionId":id,"prompt":[{"type":"text","text":text}]}),
    )
    .await
}

#[tokio::test]
async fn lifecycle_recovers_disk_history_with_same_id_and_provider_context() {
    let root = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let peer = RecordingPeer::default();
    let adapter = durable_adapter(root.path(), requests.clone());
    let new = rpc(
        &adapter,
        &peer,
        "session/new",
        serde_json::json!({"cwd":root.path(),"mcpServers":[]}),
    )
    .await;
    let id = new.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        prompt(&adapter, &peer, &id, "first remembered message")
            .await
            .result
            .unwrap()["stopReason"],
        "end_turn"
    );
    assert!(
        rpc(
            &adapter,
            &peer,
            "session/close",
            serde_json::json!({"sessionId":id})
        )
        .await
        .error
        .is_none()
    );
    assert!(prompt(&adapter, &peer, &id, "closed").await.error.is_some());
    drop(adapter);
    // A completely new runtime, app-server and adapter share only the on-disk store.
    let adapter = durable_adapter(root.path(), requests.clone());
    peer.notifications.lock().await.clear();
    assert!(
        rpc(&adapter, &peer, "session/load", reopen(&id, root.path()))
            .await
            .error
            .is_none()
    );
    let replay = peer.notifications.lock().await.clone();
    assert_eq!(replay[0].params["sessionId"], id);
    assert_eq!(
        replay[0].params["update"]["sessionUpdate"],
        "user_message_chunk"
    );
    assert_eq!(
        replay[0].params["update"]["content"]["text"],
        "first remembered message"
    );
    assert!(
        replay
            .iter()
            .any(|n| n.params["update"]["sessionUpdate"] == "agent_message_chunk")
    );
    assert!(
        replay
            .iter()
            .all(|n| n.params["update"]["messageId"].is_string())
    );
    assert!(
        rpc(
            &adapter,
            &peer,
            "session/close",
            serde_json::json!({"sessionId":id})
        )
        .await
        .error
        .is_none()
    );
    drop(adapter);
    let adapter = durable_adapter(root.path(), requests.clone());
    peer.notifications.lock().await.clear();
    assert!(
        rpc(&adapter, &peer, "session/resume", reopen(&id, root.path()))
            .await
            .error
            .is_none()
    );
    assert!(
        peer.notifications.lock().await.is_empty(),
        "resume must not replay history"
    );
    assert!(
        prompt(&adapter, &peer, &id, "second message")
            .await
            .error
            .is_none()
    );
    let requests = requests.lock().await;
    assert!(requests.last().unwrap().transcript.iter().any(|item|
        matches!(item, TranscriptItem::UserMessage(message) if message.text == "first remembered message")));
    assert!(
        requests
            .last()
            .unwrap()
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::AssistantMessage(_)))
    );
}

#[tokio::test]
async fn lifecycle_rejects_unknown_sessions_workspace_changes_and_unadvertised_roots() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let adapter = durable_adapter(root.path(), requests);
    let peer = RecordingPeer::default();
    let id = rpc(
        &adapter,
        &peer,
        "session/new",
        serde_json::json!({"cwd":root.path(),"mcpServers":[]}),
    )
    .await
    .result
    .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    for method in ["session/load", "session/resume"] {
        assert_eq!(
            rpc(&adapter, &peer, method, reopen("missing", root.path()))
                .await
                .error
                .unwrap()
                .code,
            -32002
        );
        assert_eq!(
            rpc(&adapter, &peer, method, reopen(&id, other.path()))
                .await
                .error
                .unwrap()
                .code,
            -32602
        );
        let mut params = reopen(&id, root.path());
        params["additionalDirectories"] = serde_json::json!([other.path()]);
        assert_eq!(
            rpc(&adapter, &peer, method, params)
                .await
                .error
                .unwrap()
                .code,
            -32602
        );
        assert_eq!(
            rpc(&adapter, &peer, method, reopen(&id, Path::new("relative")))
                .await
                .error
                .unwrap()
                .code,
            -32602
        );
    }
    assert_eq!(
        rpc(
            &adapter,
            &peer,
            "session/close",
            serde_json::json!({"sessionId":"missing"})
        )
        .await
        .error
        .unwrap()
        .code,
        -32002
    );
    assert!(peer.notifications.lock().await.is_empty());
}

#[tokio::test]
async fn lifecycle_close_cancels_and_drains_a_running_prompt() {
    let (adapter, runtime) = pending_test_adapter();
    let peer = RecordingPeer::default();
    let cwd = std::env::current_dir().unwrap();
    let id = rpc(
        &adapter,
        &peer,
        "session/new",
        serde_json::json!({"cwd":cwd,"mcpServers":[]}),
    )
    .await
    .result
    .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let task_adapter = adapter.clone();
    let task_peer = peer.clone();
    let task_id = id.clone();
    let task =
        tokio::spawn(async move { prompt(&task_adapter, &task_peer, &task_id, "wait").await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.active_turn_count().await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        rpc(&adapter, &peer, "session/load", reopen(&id, &cwd))
            .await
            .error
            .unwrap()
            .code,
        -32602
    );
    assert_eq!(
        prompt(&adapter, &peer, &id, "overlapping")
            .await
            .error
            .unwrap()
            .code,
        -32602
    );
    assert!(
        rpc(
            &adapter,
            &peer,
            "session/close",
            serde_json::json!({"sessionId":id})
        )
        .await
        .error
        .is_none()
    );
    assert_eq!(
        task.await.unwrap().result.unwrap()["stopReason"],
        "cancelled"
    );
    assert_eq!(runtime.active_turn_count().await, 0);
    assert!(runtime.thread_activity(&id).await.active_turn_id.is_none());
    assert!(prompt(&adapter, &peer, &id, "closed").await.error.is_some());
}

struct ApprovalPolicy;
#[async_trait]
impl roder_api::context::PolicyContributor for ApprovalPolicy {
    fn id(&self) -> String {
        "test-approval".into()
    }
    async fn review_tool(
        &self,
        _: roder_api::context::PolicyReview,
    ) -> anyhow::Result<roder_api::context::PolicyContribution> {
        Ok(roder_api::context::PolicyContribution::RequireApproval {
            reason: Some("test permission".into()),
        })
    }
}

#[derive(Default)]
struct WaitingPeer {
    requested: tokio::sync::Notify,
}
#[async_trait]
impl AcpClientPeer for WaitingPeer {
    async fn send_notification(&self, _: JsonRpcNotification) -> anyhow::Result<()> {
        Ok(())
    }
    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        self.requested.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn lifecycle_close_does_not_wait_for_an_unanswered_permission_request() {
    let root = tempfile::tempdir().unwrap();
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(HistoryEngine(Arc::new(Mutex::new(Vec::new())))));
    builder.tool_contributor(
        roder_tools::builtin_coding_tools_contributor(root.path().to_path_buf()).unwrap(),
    );
    builder.policy_contributor(Arc::new(ApprovalPolicy));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let adapter = AcpAdapter::new(LocalAppClient::new(Arc::new(
        AppServer::with_feature_config(
            runtime.clone(),
            AppServerFeatureConfig::default()
                .with_workspace_registry_path(root.path().join("workspaces.json")),
        ),
    )));
    let peer = Arc::new(WaitingPeer::default());
    let id = rpc(
        &adapter,
        peer.as_ref(),
        "session/new",
        serde_json::json!({"cwd":root.path(),"mcpServers":[]}),
    )
    .await
    .result
    .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let a = adapter.clone();
    let p = peer.clone();
    let session = id.clone();
    let task =
        tokio::spawn(async move { prompt(&a, p.as_ref(), &session, "ask permission").await });
    // Allow tool startup under contention; the peer never answers permission.
    tokio::time::timeout(Duration::from_secs(30), peer.requested.notified())
        .await
        .expect("permission was not requested");
    assert!(
        rpc(
            &adapter,
            peer.as_ref(),
            "session/close",
            serde_json::json!({"sessionId":id})
        )
        .await
        .error
        .is_none()
    );
    assert_eq!(
        task.await.unwrap().result.unwrap()["stopReason"],
        "cancelled"
    );
    assert_eq!(runtime.active_turn_count().await, 0);
    assert!(!root.path().join("denied.txt").exists());
}

#[tokio::test]
async fn lifecycle_corrupt_history_reports_server_failure_instead_of_requesting_login() {
    let root = tempfile::tempdir().unwrap();
    let adapter = durable_adapter(root.path(), Arc::new(Mutex::new(Vec::new())));
    let peer = RecordingPeer::default();
    let id = rpc(
        &adapter,
        &peer,
        "session/new",
        serde_json::json!({"cwd":root.path(),"mcpServers":[]}),
    )
    .await
    .result
    .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(adapter);
    std::fs::write(
        root.path().join("threads").join(&id).join("metadata.json"),
        "invalid metadata",
    )
    .unwrap();
    let adapter = durable_adapter(root.path(), Arc::new(Mutex::new(Vec::new())));
    assert_eq!(
        rpc(&adapter, &peer, "session/load", reopen(&id, root.path()))
            .await
            .error
            .unwrap()
            .code,
        -32603
    );
}
