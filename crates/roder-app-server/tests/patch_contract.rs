use agent_client_protocol_schema as acp;
use futures::stream;
use roder_api::{extension::ExtensionRegistryBuilder, inference::*};
use roder_app_server::{
    AppServer, AppServerFeatureConfig, LocalAppClient,
    acp::{AcpAdapter, AcpClientPeer},
};
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::{JsonRpcNotification, JsonRpcRequest};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Mutex;

struct PatchEngine {
    calls: AtomicUsize,
    patch: String,
}
#[async_trait::async_trait]
impl InferenceEngine for PatchEngine {
    fn id(&self) -> String {
        "mock".into()
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
        _: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let mut events = vec![];
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            events.push(InferenceEvent::ToolCallStarted(ToolCallStarted {
                id: "patch-call".into(),
                name: "apply_patch".into(),
            }));
            events.push(InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "patch-call".into(),
                arguments_delta: self.patch.clone(),
            }));
            events.push(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "patch-call".into(),
                name: "apply_patch".into(),
                arguments: json!({"patch":self.patch}).to_string(),
            }));
        } else {
            events.push(InferenceEvent::MessageDelta(MessageDelta {
                text: "done".into(),
                phase: None,
            }));
        }
        events.push(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("completed".into()),
            provider_response_id: None,
        }));
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}
#[derive(Default)]
struct Peer(Mutex<Vec<JsonRpcNotification>>);
#[async_trait::async_trait]
impl AcpClientPeer for Peer {
    async fn send_notification(&self, notification: JsonRpcNotification) -> anyhow::Result<()> {
        self.0.lock().await.push(notification);
        Ok(())
    }
    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                "allow_once",
            )),
        ))
    }
}
fn rpc(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(method)),
        method: method.into(),
        params: Some(params),
    }
}
async fn exercise(verification_failure: bool) {
    let dir = tempfile::tempdir().unwrap();
    let patch = if verification_failure {
        "*** Begin Patch\n*** Add File: result.txt\n+hello\n*** Delete File: missing.txt\n*** End Patch"
    } else {
        "*** Begin Patch\n*** Add File: result.txt\n+hello\n*** End Patch"
    };
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(Arc::new(PatchEngine {
        calls: AtomicUsize::new(0),
        patch: patch.into(),
    }));
    registry.tool_contributor(roder_tools::builtin_coding_tools_contributor(dir.path()).unwrap());
    let config = RuntimeConfig {
        workspace: Some(dir.path().to_string_lossy().into()),
        policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
        tool_allowlist: vec!["apply_patch".into()],
        ..RuntimeConfig::default()
    };
    let runtime = Arc::new(Runtime::new(registry.build().unwrap(), config).unwrap());
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig::default()
            .with_workspace_registry_path(dir.path().join("registry.json")),
    ));
    let client = LocalAppClient::new(server);
    let mut native = client.subscribe_notifications();
    let adapter = AcpAdapter::new(client);
    let peer = Peer::default();
    let session = adapter
        .handle_request(
            rpc(
                "session/new",
                serde_json::to_value(acp::NewSessionRequest::new(dir.path())).unwrap(),
            ),
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    let session_id = session.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let prompt = acp::PromptRequest::new(
        session_id,
        vec![acp::ContentBlock::Text(acp::TextContent::new(
            "apply the patch",
        ))],
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        adapter.handle_request(
            rpc("session/prompt", serde_json::to_value(prompt).unwrap()),
            &peer,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(response.error.is_none(), "{response:?}");
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
    let mut native_events = vec![];
    while let Ok(event) = native.try_recv() {
        native_events.push(event);
    }
    let progress = native_events
        .iter()
        .find(|event| {
            event.method == "item/applyPatch/progress" && event.params["complete"] == true
        })
        .expect("public patch progress missing");
    assert_eq!(progress.params["toolId"], "patch-call");
    assert_eq!(progress.params["patch"], patch);
    assert_eq!(progress.params["changes"][0]["newLines"], json!(["hello"]));
    let updates = peer.0.lock().await;
    assert!(updates.iter().any(|event| event.method == "session/update"
        && event.params["update"]["sessionUpdate"] == "tool_call_update"
        && event.params["update"]["toolCallId"] == "patch-call"
        && event.params["update"]["rawInput"]["patch"] == patch));
    let expected_status = if verification_failure {
        "failed"
    } else {
        "completed"
    };
    assert!(
        updates
            .iter()
            .any(|event| event.params["update"]["toolCallId"] == "patch-call"
                && event.params["update"]["status"] == expected_status),
        "missing tool result: {updates:?}"
    );
    if verification_failure {
        assert!(!dir.path().join("result.txt").exists());
        assert!(
            !native_events
                .iter()
                .any(|event| event.method == "hunk/recorded")
        );
    } else {
        assert_eq!(
            std::fs::read_to_string(dir.path().join("result.txt")).unwrap(),
            "hello\n"
        );
        assert!(
            native_events
                .iter()
                .any(|event| event.method == "hunk/recorded")
        );
    }
}
#[tokio::test]
async fn patch_progress_and_actual_change_cross_native_and_acp_boundaries() {
    exercise(false).await;
}
#[tokio::test]
async fn proposed_progress_does_not_claim_writes_when_patch_verification_fails() {
    exercise(true).await;
}
