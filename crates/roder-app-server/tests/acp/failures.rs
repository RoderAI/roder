use super::*;

struct FailingEngine {
    fail_before_stream: bool,
}

#[async_trait]
impl InferenceEngine for FailingEngine {
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
        if self.fail_before_stream {
            anyhow::bail!("test inference provider failed before stream");
        }
        Ok(Box::pin(stream::iter(vec![Ok(
            roder_api::inference::InferenceEvent::Failed(roder_api::inference::InferenceFailure {
                message: "test inference provider failed".into(),
            }),
        )])))
    }
}

#[tokio::test]
async fn failed_native_turn_returns_acp_error_and_releases_session() {
    exercise_failure(false).await;
}

#[tokio::test]
async fn provider_startup_failure_returns_acp_error_and_releases_session() {
    exercise_failure(true).await;
}

async fn exercise_failure(fail_before_stream: bool) {
    let registry_path = std::env::temp_dir().join(format!(
        "roder-acp-failure-workspaces-{}.json",
        uuid::Uuid::new_v4()
    ));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FailingEngine { fail_before_stream }));
    let mut config = roder_core::RuntimeConfig::default();
    config.reliability.provider_retry_max_attempts = 1;
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
    let server = Arc::new(AppServer::with_feature_config(
        runtime.clone(),
        AppServerFeatureConfig::default().with_workspace_registry_path(registry_path),
    ));
    let adapter = AcpAdapter::new(LocalAppClient::new(server));
    let peer = RecordingPeer::default();
    let response = adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(serde_json::json!("new")),
                method: "session/new".into(),
                params: Some(serde_json::json!({
                    "cwd": std::env::current_dir().unwrap(),
                    "mcpServers": [],
                })),
            },
            &peer,
        )
        .await
        .unwrap()
        .unwrap();
    let session_id = response.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    // A second prompt must reach the provider, not leave the session busy.
    let mut previous_turn_id = None;
    for id in [1, 2] {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            adapter.handle_request(
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(serde_json::json!(id)),
                    method: "session/prompt".into(),
                    params: Some(serde_json::json!({
                        "sessionId": session_id,
                        "prompt": [{"type": "text", "text": "hello"}],
                    })),
                },
                &peer,
            ),
        )
        .await
        .expect("failed turn must settle the ACP prompt")
        .unwrap()
        .unwrap();
        assert_eq!(response.id, Some(serde_json::json!(id)));
        assert!(response.result.is_none(), "failure is not end_turn");
        let error = response.error.expect("failed native turn must fail ACP");
        assert_eq!(error.code, -32603);
        assert!(error.message.contains("test inference provider failed"));
        let data = error.data.expect("native error context");
        assert_eq!(data["threadId"], session_id);
        assert!(data["turnId"].as_str().is_some_and(|id| !id.is_empty()));
        assert_ne!(previous_turn_id.as_ref(), Some(&data["turnId"]));
        previous_turn_id = Some(data["turnId"].clone());
        assert_eq!(runtime.active_turn_count().await, 0);
        assert!(
            data["error"]["message"]
                .as_str()
                .unwrap()
                .contains("test inference provider failed")
        );
    }
}
