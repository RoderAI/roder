use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::{StreamExt, stream};
use roder_api::catalog::{PROVIDER_MOCK, models_for_provider};
use roder_api::events::RoderEvent;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::goals::ThreadGoalPatch;
use roder_api::inference::*;
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;
use tokio::sync::Notify;

struct CompactionEngine {
    interrupt: bool,
    sampling_calls: AtomicUsize,
    compaction_calls: AtomicUsize,
    stalled: Notify,
}

fn normal_usage(first: bool) -> TokenUsage {
    if first {
        TokenUsage::new(20, 4, 24)
            .with_cached_prompt_tokens(2)
            .with_cache_creation_prompt_tokens(1)
            .with_service_tier(Some("priority".into()))
    } else {
        TokenUsage::new(5, 1, 6)
            .with_cached_prompt_tokens(1)
            .with_service_tier(Some("priority".into()))
    }
}

fn compaction_usage() -> TokenUsage {
    TokenUsage::new(10, 2, 12)
        .with_cached_prompt_tokens(3)
        .with_cache_creation_prompt_tokens(2)
        .with_service_tier(Some("default".into()))
}

fn completed() -> InferenceEvent {
    InferenceEvent::Completed(CompletionMetadata {
        stop_reason: Some("stop".into()),
        provider_response_id: None,
    })
}

#[async_trait::async_trait]
impl InferenceEngine for CompactionEngine {
    fn id(&self) -> String {
        PROVIDER_MOCK.into()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::text_only()
    }

    fn requires_native_compaction(&self) -> bool {
        true
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_MOCK, true))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let first = self.sampling_calls.fetch_add(1, Ordering::SeqCst) == 0;
        let mut events = vec![
            Ok(InferenceEvent::Usage(normal_usage(first))),
            Ok(InferenceEvent::MessageDelta(MessageDelta {
                text: if first {
                    "evidence ".repeat(2_000)
                } else {
                    "done".into()
                },
                phase: None,
            })),
        ];
        if first {
            events.push(Ok(InferenceEvent::ProviderMetadata(
                json!({"end_turn":false}),
            )));
        }
        events.push(Ok(completed()));
        Ok(Box::pin(stream::iter(events)))
    }

    async fn compact_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        _request: AgentInferenceRequest,
    ) -> anyhow::Result<Option<InferenceEventStream>> {
        self.compaction_calls.fetch_add(1, Ordering::SeqCst);
        let mut events = vec![
            Ok(InferenceEvent::Usage(TokenUsage::new(3, 1, 4))),
            Ok(InferenceEvent::Usage(compaction_usage())),
        ];
        if self.interrupt {
            // The test also waits for the goal charge before interrupting.
            self.stalled.notify_one();
            return Ok(Some(Box::pin(
                stream::iter(events).chain(stream::pending()),
            )));
        }
        events.push(Ok(InferenceEvent::ProviderMetadata(json!({
            "output": [{"type":"compaction","encrypted_content":"opaque"}],
            "compacted_input": [{"type":"compaction","encrypted_content":"opaque"}]
        }))));
        events.push(Ok(completed()));
        Ok(Some(Box::pin(stream::iter(events))))
    }
}

async fn exercise(interrupt: bool) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Arc::new(CompactionEngine {
        interrupt,
        sampling_calls: AtomicUsize::new(0),
        compaction_calls: AtomicUsize::new(0),
        stalled: Notify::new(),
    });
    let mut registry = ExtensionRegistryBuilder::new();
    registry.inference_engine(engine.clone());
    registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().join("threads"),
    }));
    let runtime = Arc::new(
        Runtime::new(
            registry.build().unwrap(),
            RuntimeConfig {
                default_provider: PROVIDER_MOCK.into(),
                default_model: "mock".into(),
                auto_compact_token_limit: Some(1_000),
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    runtime
        .thread_goal_set(
            &thread,
            ThreadGoalPatch {
                objective: Some("Exercise compaction accounting".into()),
                token_budget: Some(Some(42)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut events = runtime.subscribe_events();
    let turn = runtime
        .start_turn(StartTurnRequest {
            thread_id: thread.clone(),
            message: "Inspect the evidence".into(),
            images: Vec::new(),
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: directory.path().display().to_string(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    if interrupt {
        tokio::time::timeout(Duration::from_secs(10), engine.stalled.notified())
            .await
            .expect("native compaction should start");
        // A persisted goal update confirms that the final compaction snapshot
        // was handled; merely returning the stream does not guarantee this.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let goal = runtime.thread_goal_get(&thread).await.unwrap().unwrap();
                if goal.tokens_used >= 36 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("compaction usage should be observed");
        runtime
            .interrupt_turn(thread.clone(), turn.clone())
            .await
            .unwrap();
    }
    let terminal = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let envelope = events.recv().await.unwrap();
            if envelope.turn_id.as_deref() != Some(turn.as_str()) {
                continue;
            }
            match envelope.event {
                RoderEvent::TurnInterrupted(event) if interrupt => break event.usage,
                RoderEvent::TurnCompleted(event) if !interrupt => break event.usage,
                RoderEvent::TurnFailed(event) => panic!("turn failed: {}", event.error),
                _ => {}
            }
        }
    })
    .await
    .expect("terminal event");
    runtime.drain_active_turns(Duration::from_secs(5)).await;
    let mut expected = normal_usage(true);
    expected.add_assign(&compaction_usage());
    if !interrupt {
        expected.add_assign(&normal_usage(false));
    }
    assert_eq!(terminal, Some(expected.clone()));
    let metadata = runtime
        .load_thread(&thread)
        .await
        .unwrap()
        .unwrap()
        .metadata
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(metadata.prompt_tokens, u64::from(expected.prompt_tokens));
    assert_eq!(
        metadata.completion_tokens,
        u64::from(expected.completion_tokens)
    );
    assert_eq!(metadata.total_tokens, u64::from(expected.total_tokens));
    assert_eq!(
        metadata.cached_prompt_tokens,
        u64::from(expected.cached_prompt_tokens)
    );
    assert_eq!(
        metadata.cache_creation_prompt_tokens,
        u64::from(expected.cache_creation_prompt_tokens)
    );
    let goal = runtime.thread_goal_get(&thread).await.unwrap().unwrap();
    assert_eq!(goal.tokens_used, i64::from(expected.total_tokens));
    assert_eq!(engine.compaction_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        engine.sampling_calls.load(Ordering::SeqCst),
        if interrupt { 1 } else { 2 }
    );
}

#[tokio::test]
async fn interrupted_native_compaction_preserves_observed_usage_once() {
    exercise(true).await;
}

#[tokio::test]
async fn normal_and_native_compaction_rounds_share_terminal_accounting() {
    exercise(false).await;
}
