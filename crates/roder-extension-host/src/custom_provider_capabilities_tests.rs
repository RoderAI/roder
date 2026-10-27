use super::*;
use roder_api::inference::{AgentInferenceRequest, InferenceTurnContext, ModelSelection};
use roder_api::provider_capabilities::{RemoteCompaction, ResponsesProviderCapabilities};

#[tokio::test]
async fn registry_propagates_custom_native_compaction_opt_in() {
    let registry = build_default_registry(DefaultRegistryConfig {
        custom_inference_providers: vec![CustomInferenceProviderConfig {
            id: "custom-v2".into(),
            name: None,
            api_key: None,
            base_url: "https://custom.example/v1".into(),
            capabilities: ResponsesProviderCapabilities {
                remote_compaction: Some(RemoteCompaction::V2),
                external_web_access: Some(false),
            },
        }],
        ..Default::default()
    })
    .unwrap();
    let provider = registry.inference_engine("custom-v2").unwrap();
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "custom-v2".into(),
            model: "custom-model".into(),
        },
        instructions: Default::default(),
        transcript: vec![],
        tools: vec![],
        tool_choice: roder_api::tools::ToolChoice::Auto,
        reasoning: Default::default(),
        output: Default::default(),
        runtime: Default::default(),
        metadata: serde_json::json!({}),
    };
    // An opted-in endpoint attempts the native protocol, even without a key.
    // Losing the config in the host would silently return unsupported instead.
    let result = provider
        .compact_turn(
            InferenceTurnContext {
                thread_id: "thread",
                turn_id: "turn",
                tool_executor: None,
            },
            request,
        )
        .await;
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("requires authentication")
    );
    assert!(!provider.requires_native_compaction());
}
