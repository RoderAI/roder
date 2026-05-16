use std::{path::PathBuf, sync::Arc};

use roder_api::catalog::PROVIDER_MOCK;
use roder_api::extension::{ExtensionRegistryBuilder, InferenceEngineId};
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceTurnContext,
    MessageDelta, ModelDescriptor,
};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::{
    CommandsRunParams, CommandsRunResult, CreateSessionResult, JsonRpcRequest,
};
use tokio::sync::Mutex;

struct CapturingEngine {
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for CapturingEngine {
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
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.lock().await.push(request);
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: "command turn complete".to_string(),
            })),
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".to_string()),
                provider_response_id: None,
            })),
        ])))
    }
}

#[tokio::test]
async fn commands_run_expands_and_starts_turn_with_model_override() {
    let workspace = temp_workspace("commands_run");
    std::fs::create_dir_all(workspace.join(".roder").join("commands")).unwrap();
    std::fs::write(workspace.join("notes.txt"), "secret-notes".repeat(8_000)).unwrap();
    std::fs::write(
        workspace.join(".roder").join("commands").join("review.md"),
        r#"---
description: Review the selected area.
argument-hint: "[area]"
model: command-model
allowed-tools: [read_file]
include:
  files:
    - id: notes
      path: notes.txt
---
Review {{arguments|default("the workspace")}}.

Notes: {{include.files.notes}}
"#,
    )
    .unwrap();

    let engine = Arc::new(CapturingEngine {
        requests: Mutex::new(Vec::new()),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(workspace.display().to_string()),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));

    let session: CreateSessionResult = request(&client, "sessions/create", None).await;
    let result: CommandsRunResult = request(
        &client,
        "commands/run",
        Some(
            serde_json::to_value(CommandsRunParams {
                thread_id: session.thread_id,
                name: "review".to_string(),
                arguments: Some("api".to_string()),
            })
            .unwrap(),
        ),
    )
    .await;

    assert_eq!(result.expanded.name, "review");
    assert_eq!(result.expanded.model.as_deref(), Some("command-model"));
    assert_eq!(result.expanded.allowed_tools, vec!["read_file"]);
    assert!(result.expanded.message.contains("Review api."));
    assert!(
        result
            .expanded
            .message
            .contains("[context:command.review.files.notes]")
    );
    assert_eq!(result.expanded.context_blocks.len(), 1);
    assert_eq!(result.expanded.context_blocks[0].text.len(), 65_536);
    assert_eq!(
        result.expanded.context_blocks[0].metadata["truncated"].as_bool(),
        Some(true)
    );

    for _ in 0..20 {
        if !engine.requests.lock().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let requests = engine.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.model, "command-model");
    assert!(format!("{:?}", requests[0].conversation).contains("Review api."));
    assert!(!format!("{:?}", requests[0].conversation).contains("secret-notessecret-notes"));
}

fn temp_workspace(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("roder-app-server-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

async fn request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(method)),
        method: method.to_string(),
        params,
    };
    let res = client.send_request(req).await;
    assert!(
        res.error.is_none(),
        "RPC error for {method}: {:?}",
        res.error
    );
    serde_json::from_value(res.result.unwrap()).unwrap()
}
