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
    assert!(
        compacted
            .iter()
            .all(crate::compaction::is_compaction_boundary)
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
async fn pre_turn_compaction_uses_resolved_provider_instead_of_runtime_default() {
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
        .transcript_for_turn(&request, &"turn".into(), "openai", "gpt-5.5")
        .await
        .unwrap();
    assert!(
        transcript
            .iter()
            .all(crate::compaction::is_compaction_boundary)
    );
    assert_eq!(engine.requests.lock().unwrap().len(), 1);
}
