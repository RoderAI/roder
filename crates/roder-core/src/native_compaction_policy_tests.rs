use std::sync::{Arc, Mutex};

use futures::stream;
use roder_api::{
    extension::ExtensionRegistryBuilder,
    inference::*,
    provider_error::{ProviderFailure, ProviderFailureKind},
    transcript::{ToolResultRecord, TranscriptItem, UserMessage},
};
use serde_json::json;

use crate::{Runtime, RuntimeConfig, compaction::CompactionOptions};

#[derive(Clone, Copy)]
enum Outcome {
    Complete,
    Unsupported,
    TooLarge,
}

struct Engine {
    outcome: Outcome,
    requests: Mutex<Vec<AgentInferenceRequest>>,
}

#[async_trait::async_trait]
impl InferenceEngine for Engine {
    fn id(&self) -> String {
        "openai".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }
    fn requires_native_compaction(&self) -> bool {
        true
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
        panic!("OpenAI must never use Roder summary inference")
    }
    async fn compact_turn(
        &self,
        _: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        self.requests.lock().unwrap().push(request);
        match self.outcome {
            Outcome::Unsupported => Ok(None),
            Outcome::TooLarge => Err(ProviderFailure::new(
                ProviderFailureKind::ContextWindowExceeded,
                "native context too large",
            )
            .into()),
            Outcome::Complete => Ok(Some(Box::pin(stream::iter(vec![
                Ok(InferenceEvent::ProviderMetadata(json!({
                    "output": [{"type":"compaction","encrypted_content":"opaque"}],
                    "compacted_input": [{"type":"compaction","encrypted_content":"opaque"}]
                }))),
                Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("completed".into()),
                    provider_response_id: None,
                })),
            ])))),
        }
    }
}

fn runtime(outcome: Outcome) -> (Runtime, Arc<Engine>) {
    let engine = Arc::new(Engine {
        outcome,
        requests: Mutex::new(vec![]),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    let config = RuntimeConfig {
        default_provider: "openai".into(),
        auto_compact_token_limit: Some(1_000),
        ..RuntimeConfig::default()
    };
    (
        Runtime::new(registry.build().unwrap(), config).unwrap(),
        engine,
    )
}

#[tokio::test]
async fn openai_compacts_at_watermark_and_preserves_old_tool_outputs() {
    let (runtime, engine) = runtime(Outcome::Complete);
    let transcript = vec![
        TranscriptItem::UserMessage(UserMessage::text("goal")),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "old".into(),
            name: Some("read".into()),
            result: "evidence".repeat(6_000),
            display_payload: None,
            is_error: false,
        }),
        TranscriptItem::UserMessage(UserMessage::text("continue")),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "new".into(),
            name: Some("read".into()),
            result: "recent".into(),
            display_payload: None,
            is_error: false,
        }),
    ];
    let compacted = runtime
        .compact_transcript_if_needed(
            &"thread".into(),
            &"turn".into(),
            "openai",
            "gpt-5.5",
            transcript.clone(),
            CompactionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(compacted.len(), 1);
    assert!(
        matches!(&compacted[0], TranscriptItem::ProviderMetadata(value)
        if value["compacted_input"][0]["encrypted_content"] == "opaque")
    );
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transcript, transcript);
}

#[tokio::test]
async fn openai_native_failures_never_fall_back_to_local_summaries() {
    for outcome in [Outcome::Unsupported, Outcome::TooLarge] {
        let (runtime, engine) = runtime(outcome);
        let error = runtime
            .compact_transcript_if_needed(
                &"thread".into(),
                &"turn".into(),
                "openai",
                "uncatalogued-openai-model",
                vec![TranscriptItem::UserMessage(UserMessage::text(
                    "history".repeat(1_000),
                ))],
                CompactionOptions {
                    force: true,
                    ..CompactionOptions::default()
                },
            )
            .await
            .unwrap_err();
        match outcome {
            Outcome::TooLarge => assert_eq!(
                error.downcast_ref::<ProviderFailure>().unwrap().kind,
                ProviderFailureKind::ContextWindowExceeded
            ),
            Outcome::Unsupported => {
                assert!(error.to_string().contains("requires native compaction"))
            }
            Outcome::Complete => unreachable!(),
        }
        assert_eq!(engine.requests.lock().unwrap().len(), 1);
        assert_eq!(runtime.compaction_generation(&"thread".into()), 0);
    }
}

#[tokio::test]
async fn context_assembly_defers_compaction_until_provider_selection() {
    let engine = Arc::new(Engine {
        outcome: Outcome::Complete,
        requests: Mutex::new(vec![]),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    let runtime = Runtime::new(
        registry.build().unwrap(),
        RuntimeConfig {
            auto_compact_token_limit: Some(1_000),
            ..RuntimeConfig::default()
        },
    )
    .unwrap();
    let request = crate::StartTurnRequest {
        thread_id: "thread".into(),
        message: "user goal".repeat(1_000),
        images: vec![],
        provider_override: None,
        model_override: None,
        reasoning_override: None,
        workspace: std::env::current_dir().unwrap().display().to_string(),
        instructions: InstructionBundle::default(),
        developer_context: None,
        task_ledger_required: false,
        service_tier_override: None,
    };
    assert_ne!(runtime.status().await.default_provider, "openai");
    let transcript = runtime
        .transcript_for_turn(&request, &"turn".into())
        .await
        .unwrap();
    assert!(engine.requests.lock().unwrap().is_empty());
    let transcript = runtime
        .compact_transcript_if_needed(
            &request.thread_id,
            &"turn".into(),
            "openai",
            "gpt-5.5",
            transcript,
            CompactionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(transcript.len(), 1);
    assert!(
        matches!(&transcript[0], TranscriptItem::ProviderMetadata(value)
        if value["compacted_input"][0]["encrypted_content"] == "opaque")
    );
    assert_eq!(engine.requests.lock().unwrap().len(), 1);
}

struct RouteToMock;

#[async_trait::async_trait]
impl roder_api::inference_routing::InferenceRouter for RouteToMock {
    fn id(&self) -> String {
        "test-router".into()
    }
    async fn route(
        &self,
        _: roder_api::inference_routing::InferenceRoutingContext,
    ) -> anyhow::Result<roder_api::inference_routing::InferenceRoutingDecision> {
        Ok(
            roder_api::inference_routing::InferenceRoutingDecision::selected(
                "test-router",
                ModelSelection {
                    provider: "mock".into(),
                    model: "mock".into(),
                },
                "use mock",
            ),
        )
    }
}

#[tokio::test]
async fn routing_avoids_failing_native_compaction_on_default_provider() {
    use roder_api::{events::RoderEvent, inference_routing::ModelSelectionMode};
    let engine = Arc::new(Engine {
        outcome: Outcome::TooLarge,
        requests: Mutex::new(vec![]),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    registry.inference_router(Arc::new(RouteToMock));
    let root =
        std::env::temp_dir().join(format!("roder-compaction-route-{}", uuid::Uuid::new_v4()));
    registry.thread_store_factory(Arc::new(
        roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory {
            base_path: root.clone(),
        },
    ));
    let runtime = Arc::new(
        Runtime::new(
            registry.build().unwrap(),
            RuntimeConfig {
                default_provider: "openai".into(),
                default_model: "gpt-5.5".into(),
                auto_compact_token_limit: Some(1_000),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let workspace = root.display().to_string();
    let thread = runtime
        .create_thread_with(crate::CreateThreadRequest {
            title: None,
            workspace: workspace.clone(),
            workspace_id: None,
            root_id: None,
            provider: Some("openai".into()),
            model: Some("gpt-5.5".into()),
            tool_allowlist: vec![],
            developer_instructions: None,
            external_tools: vec![],
            selection_mode: Some(ModelSelectionMode::auto(
                "auto",
                "test-router",
                "Auto",
                ModelSelection {
                    provider: "openai".into(),
                    model: "gpt-5.5".into(),
                },
                None,
                None,
            )),
            runner: None,
        })
        .await
        .unwrap()
        .thread_id;
    let mut events = runtime.subscribe_events();
    let turn = runtime
        .start_turn(crate::StartTurnRequest {
            thread_id: thread,
            message: "history ".repeat(1_000),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace,
            instructions: Default::default(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if envelope.turn_id.as_deref() != Some(&turn) {
                continue;
            }
            match envelope.event {
                RoderEvent::TurnCompleted(_) => break,
                RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(engine.requests.lock().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
