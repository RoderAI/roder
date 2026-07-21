use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol_schema as acp;
use async_trait::async_trait;
use futures::stream;
use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceTurnContext, ModelDescriptor,
};
use roder_app_server::acp::{AcpAdapter, AcpClientPeer};
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::Runtime;
use roder_protocol::{JsonRpcNotification, JsonRpcRequest};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
struct RecordingPeer {
    notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
}

struct PendingEngine;

#[async_trait]
impl InferenceEngine for PendingEngine {
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

#[async_trait]
impl AcpClientPeer for RecordingPeer {
    async fn send_notification(&self, notification: JsonRpcNotification) -> anyhow::Result<()> {
        self.notifications.lock().await.push(notification);
        Ok(())
    }

    async fn request_permission(
        &self,
        _request: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                "allow_once",
            )),
        ))
    }
}

#[tokio::test]
async fn acp_initialize_advertises_stable_v1_without_optional_capabilities() {
    let adapter = test_adapter();
    let peer = RecordingPeer::default();

    let response = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("init")),
                method: "initialize".to_string(),
                params: Some(serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {}
                })),
            },
            &peer,
        )
        .await
        .unwrap()
        .expect("initialize response");

    let result = response.result.expect("initialize result");
    assert_eq!(result["protocolVersion"], 1);
    assert_eq!(result["agentCapabilities"]["loadSession"], false);
    assert!(
        result["agentCapabilities"]["sessionCapabilities"]
            .get("list")
            .is_none()
    );
    assert_eq!(
        result["agentCapabilities"]["promptCapabilities"],
        serde_json::json!({
            "image": false,
            "audio": false,
            "embeddedContext": false
        })
    );
}

#[tokio::test]
async fn acp_session_new_and_prompt_stream_session_updates() {
    let adapter = test_adapter();
    let peer = RecordingPeer::default();
    let cwd = std::env::current_dir().unwrap();

    let session = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("new")),
                method: "session/new".to_string(),
                params: Some(serde_json::to_value(acp::NewSessionRequest::new(cwd)).unwrap()),
            },
            &peer,
        )
        .await
        .unwrap()
        .expect("session/new response");
    let session_id = session
        .result
        .as_ref()
        .and_then(|value| value.get("sessionId"))
        .and_then(serde_json::Value::as_str)
        .expect("session id")
        .to_string();

    let prompt = acp::PromptRequest::new(
        session_id.clone(),
        vec![acp::ContentBlock::Text(acp::TextContent::new("hello"))],
    );
    let response = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("prompt")),
                method: "session/prompt".to_string(),
                params: Some(serde_json::to_value(prompt).unwrap()),
            },
            &peer,
        )
        .await
        .unwrap()
        .expect("session/prompt response");

    assert_eq!(
        response.result.expect("prompt result")["stopReason"],
        "end_turn"
    );
    let notifications = peer.notifications.lock().await;
    assert!(
        notifications
            .iter()
            .any(|notification| notification.method == "session/update"
                && notification.params["sessionId"] == session_id
                && notification.params["update"]["sessionUpdate"] == "agent_message_chunk"
                && notification.params["update"]["content"]["type"] == "text"),
        "missing ACP agent_message_chunk update: {notifications:?}"
    );
}

#[tokio::test]
async fn acp_session_cancel_returns_cancelled_prompt_stop_reason() {
    let (adapter, runtime) = pending_test_adapter();
    let peer = RecordingPeer::default();
    let cwd = std::env::current_dir().unwrap();

    let session = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("new")),
                method: "session/new".to_string(),
                params: Some(serde_json::to_value(acp::NewSessionRequest::new(cwd)).unwrap()),
            },
            &peer,
        )
        .await
        .unwrap()
        .expect("session/new response");
    let session_id = session
        .result
        .as_ref()
        .and_then(|value| value.get("sessionId"))
        .and_then(serde_json::Value::as_str)
        .expect("session id")
        .to_string();

    let prompt_adapter = adapter.clone();
    let prompt_peer = peer.clone();
    let prompt = acp::PromptRequest::new(
        session_id.clone(),
        vec![acp::ContentBlock::Text(acp::TextContent::new("wait"))],
    );
    let prompt_task = tokio::spawn(async move {
        prompt_adapter
            .handle_request(
                JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("prompt")),
                    method: "session/prompt".to_string(),
                    params: Some(serde_json::to_value(prompt).unwrap()),
                },
                &prompt_peer,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.active_turn_count().await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ACP prompt did not start a turn");

    let cancel_response = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: None,
                method: "session/cancel".to_string(),
                params: Some(
                    serde_json::to_value(acp::CancelNotification::new(session_id)).unwrap(),
                ),
            },
            &peer,
        )
        .await
        .unwrap();
    assert!(
        cancel_response.is_none(),
        "session/cancel is an ACP notification"
    );

    let response = tokio::time::timeout(Duration::from_secs(2), prompt_task)
        .await
        .expect("ACP prompt did not finish after cancellation")
        .expect("prompt task panicked")
        .expect("ACP adapter error")
        .expect("session/prompt response");
    assert!(
        response.error.is_none(),
        "unexpected ACP error: {response:?}"
    );
    assert_eq!(
        response.result.expect("prompt result")["stopReason"],
        "cancelled"
    );
    assert_eq!(runtime.active_turn_count().await, 0);
}

#[tokio::test]
async fn acp_rejects_unadvertised_optional_methods() {
    let adapter = test_adapter();
    let peer = RecordingPeer::default();

    let response = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("list")),
                method: "session/list".to_string(),
                params: Some(serde_json::json!({})),
            },
            &peer,
        )
        .await
        .unwrap()
        .expect("session/list error response");

    let error = response.error.expect("method-not-found error");
    assert_eq!(error.code, -32601);
}

fn test_adapter() -> AcpAdapter<LocalAppClient> {
    let registry_path = std::env::temp_dir().join(format!(
        "roder-acp-workspaces-{}.json",
        uuid::Uuid::new_v4()
    ));
    let runtime = Arc::new(Runtime::fake().unwrap());
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig::default().with_workspace_registry_path(registry_path),
    ));
    AcpAdapter::new(LocalAppClient::new(server))
}

fn pending_test_adapter() -> (AcpAdapter<LocalAppClient>, Arc<Runtime>) {
    let registry_path = std::env::temp_dir().join(format!(
        "roder-acp-workspaces-{}.json",
        uuid::Uuid::new_v4()
    ));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(PendingEngine));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let server = Arc::new(AppServer::with_feature_config(
        runtime.clone(),
        AppServerFeatureConfig::default().with_workspace_registry_path(registry_path),
    ));
    (AcpAdapter::new(LocalAppClient::new(server)), runtime)
}
