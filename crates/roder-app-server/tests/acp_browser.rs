use super::*;
use roder_api::context::{PolicyContribution, PolicyContributor, PolicyReview};
use roder_api::inference::{CompletionMetadata, InferenceEvent, ToolCallCompleted};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

struct BrowserEngine;
#[async_trait]
impl InferenceEngine for BrowserEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_MOCK.into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(Vec::new())
    }
    async fn stream_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let completed = request
            .transcript
            .iter()
            .any(|item| matches!(item, roder_api::transcript::TranscriptItem::ToolResult(_)));
        let mut events = Vec::new();
        if !completed {
            events.push(Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "browser-call".into(),
                name: "browser_fixture".into(),
                arguments: "{}".into(),
            })));
        }
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some(if completed { "stop" } else { "tool_calls" }.into()),
            provider_response_id: None,
        })));
        Ok(Box::pin(stream::iter(events)))
    }
}
struct BrowserFixture(Arc<AtomicUsize>);
#[async_trait]
impl ToolExecutor for BrowserFixture {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "browser_fixture".into(),
            description: "Browser fixture with fresh state".into(),
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: "UNTRUSTED page state: Filters open, search contains penguin".into(),
            data: json!({"__view_image":{"image_url":"data:image/png;base64,YWJj","detail":"original"}}),
            is_error: false,
        })
    }
}
struct BrowserTools(Arc<AtomicUsize>);
impl ToolContributor for BrowserTools {
    fn id(&self) -> String {
        "browser-fixture".into()
    }
    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(BrowserFixture(self.0.clone())))
    }
}
struct BrowserPolicy;
#[async_trait]
impl PolicyContributor for BrowserPolicy {
    fn id(&self) -> String {
        "browser-fixture".into()
    }
    async fn review_tool(&self, review: PolicyReview) -> anyhow::Result<PolicyContribution> {
        Ok(if review.call.name == "browser_fixture" {
            PolicyContribution::RequireApproval {
                reason: Some("Approve exact browser action".into()),
            }
        } else {
            PolicyContribution::Abstain
        })
    }
}
#[derive(Clone)]
struct BrowserPeer {
    allow: bool,
    permissions: Arc<Mutex<Vec<Value>>>,
    notifications: Arc<Mutex<Vec<JsonRpcNotification>>>,
}
#[async_trait]
impl AcpClientPeer for BrowserPeer {
    async fn send_notification(&self, notification: JsonRpcNotification) -> anyhow::Result<()> {
        self.notifications.lock().await.push(notification);
        Ok(())
    }
    async fn request_permission(
        &self,
        request: acp::RequestPermissionRequest,
    ) -> anyhow::Result<acp::RequestPermissionResponse> {
        self.permissions
            .lock()
            .await
            .push(serde_json::to_value(request)?);
        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                if self.allow {
                    "allow_once"
                } else {
                    "reject_once"
                },
            )),
        ))
    }
}
async fn exercise_browser(allow: bool) {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(BrowserEngine));
    builder.tool_contributor(Arc::new(BrowserTools(executions.clone())));
    builder.policy_contributor(Arc::new(BrowserPolicy));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    let path =
        std::env::temp_dir().join(format!("roder-acp-browser-{}.json", uuid::Uuid::new_v4()));
    let server = Arc::new(AppServer::with_feature_config(
        runtime,
        AppServerFeatureConfig::default().with_workspace_registry_path(path),
    ));
    let adapter = AcpAdapter::new(LocalAppClient::new(server));
    let peer = BrowserPeer {
        allow,
        permissions: Default::default(),
        notifications: Default::default(),
    };
    let request = |method: &str, params: Value| JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(method)),
        method: method.into(),
        params: Some(params),
    };
    let new = adapter
        .handle_request(
            request(
                "session/new",
                serde_json::to_value(acp::NewSessionRequest::new(
                    std::env::current_dir().unwrap(),
                ))
                .unwrap(),
            ),
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    let id = new.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        adapter.handle_request(
            request(
                "session/prompt",
                serde_json::to_value(acp::PromptRequest::new(
                    id.clone(),
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "Run the browser fixture",
                    ))],
                ))
                .unwrap(),
            ),
            &peer,
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(response.result.unwrap()["stopReason"], "end_turn");
    let permissions = peer.permissions.lock().await;
    assert_eq!(permissions.len(), 1);
    assert_eq!(permissions[0]["sessionId"], id);
    assert_eq!(permissions[0]["toolCall"]["toolCallId"], "browser-call");
    let notifications = peer.notifications.lock().await;
    let updates: Vec<_> = notifications
        .iter()
        .filter(|n| n.method == "session/update")
        .map(|n| &n.params["update"])
        .collect();
    assert!(
        updates.iter().any(|u| u["sessionUpdate"] == "tool_call"
            && u["toolCallId"] == "browser-call"
            && u["rawInput"] == json!({})),
        "{updates:?}"
    );
    let finished = updates
        .iter()
        .find(|u| {
            u["sessionUpdate"] == "tool_call_update"
                && u["toolCallId"] == "browser-call"
                && u["status"] == if allow { "completed" } else { "failed" }
        })
        .expect("completed ACP tool update");
    if allow {
        assert!(
            finished
                .to_string()
                .contains("Filters open, search contains penguin")
        );
    }
    assert_eq!(executions.load(Ordering::SeqCst), usize::from(allow));
}
#[tokio::test]
async fn browser_observation_and_permission_survive_acp_wire() {
    exercise_browser(true).await;
}
#[tokio::test]
async fn rejected_browser_permission_never_executes_the_action() {
    exercise_browser(false).await;
}
