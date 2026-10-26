use super::*;
use roder_api::inference::{CompletionMetadata, InferenceEvent, ToolCallCompleted};
use roder_api::transcript::TranscriptItem;
use roder_core::fake_provider::FakeInferenceEngine;
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::{Value, json};
use std::path::Path;

#[path = "mcp_cancellation.rs"]
mod mcp_cancellation;
#[path = "mcp_fixture.rs"]
mod mcp_fixture;

struct CaptureEngine(Arc<Mutex<Vec<AgentInferenceRequest>>>);

#[async_trait]
impl InferenceEngine for CaptureEngine {
    fn id(&self) -> String {
        PROVIDER_MOCK.into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            image_input: true,
            ..InferenceCapabilities::coding_agent_default()
        }
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(["mock", "other"]
            .into_iter()
            .map(|id| ModelDescriptor {
                id: id.into(),
                name: id.into(),
                context_window: Some(32000),
                default_reasoning: None,
                supported_reasoning: vec![],
            })
            .collect())
    }
    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.0.lock().await.push(request.clone());
        let tool_result = request
            .transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::ToolResult(_)));
        let mcp_prompt = request.transcript.iter().any(
            |item| matches!(item, TranscriptItem::UserMessage(message) if message.text == "MCP"),
        );
        if mcp_prompt && !tool_result {
            return Ok(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "call-echo".into(),
                    name: "mcp__demo__echo".into(),
                    arguments: json!({"value":"hello"}).to_string(),
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

fn adapter(
    root: &Path,
    captures: Arc<Mutex<Vec<AgentInferenceRequest>>>,
) -> AcpAdapter<LocalAppClient> {
    adapter_with_mode(root, captures, roder_api::policy_mode::PolicyMode::Default)
}

fn adapter_with_mode(
    root: &Path,
    captures: Arc<Mutex<Vec<AgentInferenceRequest>>>,
    mode: roder_api::policy_mode::PolicyMode,
) -> AcpAdapter<LocalAppClient> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(CaptureEngine(captures)));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: root.join("threads"),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            roder_core::RuntimeConfig {
                policy_mode: mode,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let config = AppServerFeatureConfig::default()
        .with_workspace_registry_path(root.join("workspaces.json"));
    AcpAdapter::new(LocalAppClient::new(Arc::new(
        AppServer::with_feature_config(runtime, config),
    )))
}

async fn rpc(
    adapter: &AcpAdapter<LocalAppClient>,
    peer: &impl AcpClientPeer,
    method: &str,
    params: Value,
) -> roder_protocol::JsonRpcResponse {
    tokio::time::timeout(
        Duration::from_secs(15),
        adapter.handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(json!(method)),
                method: method.into(),
                params: Some(params),
            },
            peer,
        ),
    )
    .await
    .expect("ACP timeout")
    .unwrap()
    .unwrap()
}

async fn new_session(
    adapter: &AcpAdapter<LocalAppClient>,
    peer: &impl AcpClientPeer,
    params: Value,
) -> Value {
    let response = rpc(adapter, peer, "session/new", params).await;
    assert!(response.error.is_none(), "{response:?}");
    response.result.unwrap()
}

async fn prompt(
    adapter: &AcpAdapter<LocalAppClient>,
    peer: &impl AcpClientPeer,
    id: &str,
    content: Value,
) {
    let response = rpc(
        adapter,
        peer,
        "session/prompt",
        json!({"sessionId":id,"prompt":content}),
    )
    .await;
    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
}

#[tokio::test]
async fn models_and_modes_are_scoped_and_survive_cold_restore() {
    let root = tempfile::tempdir().unwrap();
    let captures = Arc::new(Mutex::new(vec![]));
    let first = adapter(root.path(), captures.clone());
    let peer = RecordingPeer::default();
    let a = new_session(&first, &peer, json!({"cwd":root.path(),"mcpServers":[]})).await;
    let b = new_session(&first, &peer, json!({"cwd":root.path(),"mcpServers":[]})).await;
    let id = a["sessionId"].as_str().unwrap();
    assert_eq!(a["configOptions"][0]["currentValue"], "mock/mock");
    let selected = rpc(
        &first,
        &peer,
        "session/set_config_option",
        json!({"sessionId":id,"configId":"model","value":"mock/other"}),
    )
    .await;
    assert!(selected.error.is_none(), "{selected:?}");
    assert_eq!(
        selected.result.unwrap()["configOptions"][0]["currentValue"],
        "mock/other"
    );
    let mode = rpc(
        &first,
        &peer,
        "session/set_mode",
        json!({"sessionId":id,"modeId":"plan"}),
    )
    .await;
    assert!(mode.error.is_none(), "{mode:?}");
    prompt(&first, &peer, id, json!([{"type":"text","text":"hello"}])).await;
    assert_eq!(captures.lock().await.last().unwrap().model.model, "other");
    let b_state = rpc(
        &first,
        &peer,
        "session/load",
        json!({"sessionId":b["sessionId"],"cwd":root.path(),"mcpServers":[]}),
    )
    .await;
    assert_eq!(b_state.result.unwrap()["modes"]["currentModeId"], "default");
    let bad = rpc(
        &first,
        &peer,
        "session/set_config_option",
        json!({"sessionId":id,"configId":"model","value":"missing/model"}),
    )
    .await;
    assert_eq!(bad.error.unwrap().code, -32602);
    first.shutdown().await.unwrap();
    drop(first);
    let cold = adapter_with_mode(
        root.path(),
        captures,
        roder_api::policy_mode::PolicyMode::Bypass,
    );
    let loaded = rpc(
        &cold,
        &peer,
        "session/load",
        json!({"sessionId":id,"cwd":root.path(),"mcpServers":[]}),
    )
    .await;
    assert!(loaded.error.is_none(), "{loaded:?}");
    let state = loaded.result.unwrap();
    assert_eq!(state["modes"]["currentModeId"], "plan");
    assert_eq!(state["configOptions"][0]["currentValue"], "mock/other");
    let b_cold = rpc(
        &cold,
        &peer,
        "session/load",
        json!({"sessionId":b["sessionId"],"cwd":root.path(),"mcpServers":[]}),
    )
    .await;
    assert_eq!(b_cold.result.unwrap()["modes"]["currentModeId"], "default");
    assert!(peer.notifications.lock().await.iter().any(
        |notification| notification.params["update"]["sessionUpdate"] == "config_option_update"
    ));
}

#[tokio::test]
async fn images_and_paseo_host_instructions_reach_the_inference_request() {
    const IMAGE_DATA: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAADklEQVR4nGOYCQYMEAoAMkIHLdmWYfwAAAAASUVORK5CYII=";
    let root = tempfile::tempdir().unwrap();
    let captures = Arc::new(Mutex::new(vec![]));
    let adapter = adapter(root.path(), captures.clone());
    let peer = RecordingPeer::default();
    let session = new_session(&adapter, &peer, json!({"cwd":root.path(),"mcpServers":[],"_meta":{"_paseo":{"systemPrompt":"HOST_INSTRUCTIONS_SENTINEL"}}})).await;
    let id = session["sessionId"].as_str().unwrap();
    prompt(&adapter, &peer, id, json!([{"type":"text","text":"describe this"},{"type":"image","mimeType":"image/png","data":IMAGE_DATA}])).await;
    let requests = captures.lock().await;
    assert!(
        requests.iter().any(|request| request
            .instructions
            .developer
            .as_deref()
            .is_some_and(|text| text.contains("HOST_INSTRUCTIONS_SENTINEL"))),
        "{requests:#?}"
    );
    assert!(requests.iter().any(|request| request.transcript.iter().any(|item| matches!(item, TranscriptItem::UserMessage(message) if message.images.iter().any(|image| image.image_url == format!("data:image/png;base64,{IMAGE_DATA}"))))));
    drop(requests);
    let invalid = rpc(&adapter, &peer, "session/prompt", json!({"sessionId":id,"prompt":[{"type":"image","mimeType":"image/png","data":"not base64!"}]})).await;
    assert_eq!(invalid.error.unwrap().code, -32602);
    let changed = rpc(&adapter, &peer, "session/load", json!({"sessionId":id,"cwd":root.path(),"mcpServers":[],"_meta":{"_paseo":{"systemPrompt":"different"}}})).await;
    assert_eq!(changed.error.unwrap().code, -32602);
}

#[tokio::test]
async fn session_listing_filters_cwd_and_passes_pagination_cursors() {
    let root = tempfile::tempdir().unwrap();
    let adapter = adapter(root.path(), Default::default());
    let peer = RecordingPeer::default();
    for _ in 0..51 {
        new_session(&adapter, &peer, json!({"cwd":root.path(),"mcpServers":[]})).await;
    }
    let first = rpc(&adapter, &peer, "session/list", json!({"cwd":root.path()}))
        .await
        .result
        .unwrap();
    assert_eq!(first["sessions"].as_array().unwrap().len(), 50);
    let second = rpc(
        &adapter,
        &peer,
        "session/list",
        json!({"cwd":root.path(),"cursor":first["nextCursor"]}),
    )
    .await
    .result
    .unwrap();
    assert_eq!(second["sessions"].as_array().unwrap().len(), 1);
    assert!(second.get("nextCursor").is_none());
    let other = tempfile::tempdir().unwrap();
    let filtered = rpc(&adapter, &peer, "session/list", json!({"cwd":other.path()}))
        .await
        .result
        .unwrap();
    assert_eq!(filtered["sessions"], json!([]));
    let invalid = rpc(&adapter, &peer, "session/list", json!({"cwd":"relative"})).await;
    assert_eq!(invalid.error.unwrap().code, -32602);
}

#[tokio::test]
async fn injected_http_mcp_executes_with_permission_and_rebinds_after_cold_load() {
    let root = tempfile::tempdir().unwrap();
    let fixture = mcp_fixture::Server::start().await;
    let captures = Arc::new(Mutex::new(vec![]));
    let first = adapter(root.path(), captures.clone());
    let peer = RecordingPeer::default();
    let servers = fixture.config();
    let session = new_session(
        &first,
        &peer,
        json!({"cwd":root.path(),"mcpServers":servers}),
    )
    .await;
    let id = session["sessionId"].as_str().unwrap();
    prompt(&first, &peer, id, json!([{"type":"text","text":"MCP"}])).await;
    assert_eq!(
        fixture.calls.lock().await.as_slice(),
        &[json!({"value":"hello"})]
    );
    assert_eq!(*peer.permission_requests.lock().await, 1);
    assert!(captures.lock().await.iter().any(|request| {
        request
            .tools
            .iter()
            .any(|tool| tool.name == "mcp__demo__echo")
    }));
    first.shutdown().await.unwrap();
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(first);
    let cold = adapter(root.path(), captures);
    let missing = rpc(
        &cold,
        &peer,
        "session/load",
        json!({"sessionId":id,"cwd":root.path(),"mcpServers":[]}),
    )
    .await;
    assert_eq!(missing.error.unwrap().code, -32602);
    let loaded = rpc(
        &cold,
        &peer,
        "session/load",
        json!({"sessionId":id,"cwd":root.path(),"mcpServers":servers}),
    )
    .await;
    assert!(loaded.error.is_none(), "{loaded:?}");
    cold.shutdown().await.unwrap();
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 2);
}

struct RejectPeer;
#[async_trait]
impl AcpClientPeer for RejectPeer {
    async fn send_notification(&self, _: JsonRpcNotification) -> anyhow::Result<()> {
        Ok(())
    }
    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                "reject_once",
            )),
        ))
    }
}

#[tokio::test]
async fn injected_mcp_never_executes_in_plan_mode_or_after_permission_denial() {
    let root = tempfile::tempdir().unwrap();
    let fixture = mcp_fixture::Server::start().await;
    let adapter = adapter(root.path(), Default::default());
    let peer = RecordingPeer::default();
    let a = new_session(
        &adapter,
        &peer,
        json!({"cwd":root.path(),"mcpServers":fixture.config()}),
    )
    .await;
    let id = a["sessionId"].as_str().unwrap();
    let result = rpc(
        &adapter,
        &peer,
        "session/set_mode",
        json!({"sessionId":id,"modeId":"plan"}),
    )
    .await;
    assert!(result.error.is_none());
    prompt(&adapter, &peer, id, json!([{"type":"text","text":"MCP"}])).await;
    assert_eq!(*peer.permission_requests.lock().await, 0);
    let b = new_session(
        &adapter,
        &peer,
        json!({"cwd":root.path(),"mcpServers":fixture.config()}),
    )
    .await;
    prompt(
        &adapter,
        &RejectPeer,
        b["sessionId"].as_str().unwrap(),
        json!([{"type":"text","text":"MCP"}]),
    )
    .await;
    assert!(fixture.calls.lock().await.is_empty());
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn injected_stdio_mcp_receives_session_cwd_and_explicit_environment() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("server.py");
    std::fs::write(&script, mcp_fixture::STDIO_SERVER).unwrap();
    let captures = Arc::new(Mutex::new(vec![]));
    let adapter = adapter(root.path(), captures.clone());
    let peer = RecordingPeer::default();
    let session = new_session(&adapter, &peer, json!({"cwd":root.path(),"mcpServers":[{"name":"demo","command":"python3","args":[script],"env":[{"name":"ACP_SENTINEL","value":"session-value"}]}]})).await;
    prompt(
        &adapter,
        &peer,
        session["sessionId"].as_str().unwrap(),
        json!([{"type":"text","text":"MCP"}]),
    )
    .await;
    let cwd = std::fs::canonicalize(root.path()).unwrap();
    assert!(captures.lock().await.iter().any(|request| request.transcript.iter().any(|item| matches!(item, TranscriptItem::ToolResult(result) if result.result.contains("[redacted]") && result.result.contains(cwd.to_str().unwrap())))));
    adapter.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_mcp_setup_and_restore_close_opened_http_sessions() {
    use std::sync::atomic::Ordering;
    let root = tempfile::tempdir().unwrap();
    let fixture = mcp_fixture::Server::start().await;
    let adapter = adapter(root.path(), Default::default());
    let peer = RecordingPeer::default();
    let mut servers = fixture.config();
    servers.as_array_mut().unwrap().push(
        json!({"name":"broken","command":"/nonexistent/roder-mcp-fixture","args":[],"env":[]}),
    );
    let failed = rpc(
        &adapter,
        &peer,
        "session/new",
        json!({"cwd":root.path(),"mcpServers":servers}),
    )
    .await;
    assert!(failed.error.is_some());
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 1);
    let session = new_session(&adapter, &peer, json!({"cwd":root.path(),"mcpServers":[]})).await;
    let failed = rpc(
        &adapter,
        &peer,
        "session/load",
        json!({"sessionId":session["sessionId"],"cwd":root.path(),"mcpServers":fixture.config()}),
    )
    .await;
    assert_eq!(failed.error.unwrap().code, -32602);
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 2);
    // A rejected rebind preserves the original active session.
    prompt(
        &adapter,
        &peer,
        session["sessionId"].as_str().unwrap(),
        json!([{"type":"text","text":"hello"}]),
    )
    .await;
    adapter.shutdown().await.unwrap();
}
