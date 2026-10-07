use crate::runtime::Runtime;
use futures::StreamExt;
use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};
use roder_api::{
    events::{ThreadId, TurnId},
    inference::*,
    transcript::TranscriptItem,
};

pub(crate) enum NativeCompactionOutcome {
    Unsupported,
    Compacted(Vec<TranscriptItem>),
    Preempted(Vec<TranscriptItem>),
}

#[derive(Debug)]
struct CompactionPreempted;
impl std::fmt::Display for CompactionPreempted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("compaction preempted by new input")
    }
}
impl std::error::Error for CompactionPreempted {}

async fn preemptible<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
    steering: &mut Option<tokio::sync::watch::Receiver<u64>>,
) -> anyhow::Result<T> {
    if let Some(steering) = steering {
        tokio::select! { biased; _ = crate::runtime::sampling::wait_for_steer(steering) => Err(CompactionPreempted.into()), result = future => result }
    } else {
        future.await
    }
}

impl Runtime {
    pub(crate) async fn compact_with_provider(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        transcript: &[TranscriptItem],
        preserve_hint: Option<&str>,
    ) -> anyhow::Result<NativeCompactionOutcome> {
        let engine = self.engine_for(provider)?;
        let cfg = self.status().await;
        let mut request = self
            .compaction_request_template(thread_id, turn_id, provider, model)
            .await?;
        request.model = ModelSelection {
            provider: provider.into(),
            model: model.into(),
        };
        request.transcript = transcript.to_vec();
        request.runtime.auto_compact_token_limit = None;
        request.runtime.prompt_cache_key = Some(thread_id.clone());
        request.runtime.parallel_tool_calls = Some(true);
        request.runtime.reliability = Some(cfg.reliability.clone().into());
        if let Some(hint) = preserve_hint {
            let developer = request.instructions.developer.get_or_insert_default();
            developer.push_str(&format!("\nCompaction must preserve: {hint}"));
        }
        let mut steering = self.compaction_steering(turn_id).await;
        let attempts = cfg.reliability.provider_retry_max_attempts.max(1);
        let mut spent = 0u32;
        while spent < attempts {
            request
                .runtime
                .reliability
                .as_mut()
                .unwrap()
                .provider_retry_max_attempts = attempts - spent;
            let ctx = InferenceTurnContext {
                thread_id,
                turn_id,
                tool_executor: None,
            };
            let outcome: anyhow::Result<Option<Vec<TranscriptItem>>> = async {
                let Some(mut stream) =
                    preemptible(engine.compact_turn(ctx, request.clone()), &mut steering).await?
                else {
                    return Ok(None);
                };
                let mut boundary = None;
                let mut completed = false;
                while let Some(event) =
                    preemptible(async { Ok(stream.next().await) }, &mut steering).await?
                {
                    match event? {
                        InferenceEvent::ProviderMetadata(metadata) => {
                            if metadata["kind"] == "reliability_retry_attempt" {
                                spent = spent.saturating_add(1);
                            }
                            if crate::compaction::provider_metadata_has_compaction(&metadata) {
                                let item = TranscriptItem::ProviderMetadata(metadata);
                                self.persist_turn_item(thread_id, turn_id, &item).await?;
                                boundary = Some(item.clone());
                                // Commit completed opaque state before another read
                                // so a retry never restores the pre-compact window.
                                request.transcript = vec![item];
                            }
                        }
                        InferenceEvent::Usage(usage) => {
                            self.record_thread_usage_metadata(thread_id, &usage).await?;
                            self.record_goal_token_usage(
                                thread_id,
                                turn_id,
                                usage.total_tokens as i64,
                            )
                            .await?;
                        }
                        InferenceEvent::Completed(_) => {
                            completed = true;
                            break;
                        }
                        InferenceEvent::Failed(failure) => {
                            return Err(ProviderFailure::new(
                                ProviderFailureKind::Protocol,
                                failure.message,
                            )
                            .into());
                        }
                        _ => {}
                    }
                }
                if !completed {
                    return Err(ProviderFailure::new(
                        ProviderFailureKind::StreamInterrupted,
                        "native compaction ended without terminal completion",
                    )
                    .into());
                }
                let boundary = boundary.ok_or_else(|| {
                    ProviderFailure::new(
                        ProviderFailureKind::Protocol,
                        "native compaction completed without durable boundary",
                    )
                })?;
                Ok(Some(vec![boundary]))
            }
            .await;
            spent = spent.saturating_add(1);
            match outcome {
                Err(error) if error.is::<CompactionPreempted>() => {
                    return Ok(NativeCompactionOutcome::Preempted(request.transcript));
                }
                Err(error)
                    if spent < attempts
                        && error
                            .downcast_ref::<ProviderFailure>()
                            .is_some_and(|failure| {
                                failure.kind.is_retryable() && !failure.retry_budget_exhausted
                            }) =>
                {
                    let failure = error.downcast_ref::<ProviderFailure>().unwrap();
                    let policy: roder_api::reliability::ReliabilityRequestPolicy =
                        cfg.reliability.clone().into();
                    let deadline = tokio::time::Instant::now()
                        + std::time::Duration::from_millis(
                            roder_api::reliability::provider_retry_delay_ms(&policy, spent),
                        );
                    if preemptible(
                        async {
                            tokio::time::sleep_until(
                                failure
                                    .retry_not_before
                                    .map_or(deadline, |server| server.max(deadline)),
                            )
                            .await;
                            Ok(())
                        },
                        &mut steering,
                    )
                    .await
                    .is_err()
                    {
                        return Ok(NativeCompactionOutcome::Preempted(request.transcript));
                    }
                }
                Ok(Some(window)) => return Ok(NativeCompactionOutcome::Compacted(window)),
                Ok(None) => return Ok(NativeCompactionOutcome::Unsupported),
                Err(error)
                    if error
                        .downcast_ref::<ProviderFailure>()
                        .is_some_and(|failure| {
                            matches!(
                                failure.kind,
                                ProviderFailureKind::ContextWindowExceeded
                                    | ProviderFailureKind::RequestTooLarge
                            )
                        }) =>
                {
                    self.persist_turn_item(
                        thread_id,
                        turn_id,
                        &TranscriptItem::ProviderMetadata(serde_json::json!({
                        "kind":"compaction_fallback","reason":"native_window_does_not_fit"})),
                    )
                    .await?;
                    return Ok(NativeCompactionOutcome::Unsupported);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::{
        extension::ExtensionRegistryBuilder, inference_routing::ModelSelectionMode,
        transcript::UserMessage,
    };
    use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    struct NativeEngine(Mutex<Vec<AgentInferenceRequest>>);
    #[async_trait::async_trait]
    impl InferenceEngine for NativeEngine {
        fn id(&self) -> String {
            "chosen".into()
        }
        fn capabilities(&self) -> InferenceCapabilities {
            InferenceCapabilities::text_only()
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
            panic!("text-summary fallback used instead of native compaction")
        }
        async fn compact_turn(
            &self,
            ctx: InferenceTurnContext<'_>,
            request: AgentInferenceRequest,
        ) -> anyhow::Result<Option<InferenceEventStream>> {
            assert_ne!(ctx.thread_id, "compaction-summary");
            assert_eq!(request.model.provider, "chosen");
            assert_eq!(request.model.model, "selected-model");
            assert!(
                request
                    .instructions
                    .developer
                    .as_deref()
                    .unwrap()
                    .contains("preserve the goal")
            );
            let mut requests = self.0.lock().unwrap();
            requests.push(request);
            let mut events = vec![Ok(InferenceEvent::ProviderMetadata(
                json!({"output":[{"type":"compaction","id":"native1","encrypted_content":"opaque"}],
                "compacted_input":[{"type":"message","role":"user","content":"retained goal"},{"type":"compaction","id":"native1","encrypted_content":"opaque"}]}),
            ))];
            if requests.len() == 1 {
                events.push(Err(ProviderFailure::new(
                    ProviderFailureKind::StreamInterrupted,
                    "fixture disconnect",
                )
                .into()));
            } else {
                assert!(
                    requests[1]
                        .transcript
                        .iter()
                        .all(crate::compaction::is_compaction_boundary)
                );
                events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("completed".into()),
                    provider_response_id: Some("native-response".into()),
                })));
            }
            Ok(Some(Box::pin(futures::stream::iter(events))))
        }
    }
    #[tokio::test]
    async fn manual_native_compaction_uses_thread_selection_and_retries_committed_window() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(NativeEngine(Mutex::new(vec![])));
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(engine.clone());
        registry.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        registry.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
            base_path: dir.path().into(),
        }));
        let mut config = crate::RuntimeConfig::default();
        config.reliability.provider_retry_initial_backoff_ms = 0;
        let runtime = Runtime::new(registry.build().unwrap(), config).unwrap();
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        runtime
            .set_thread_selection_mode(
                &thread,
                ModelSelectionMode::manual("chosen", "selected-model", None),
            )
            .await
            .unwrap();
        let turn = "compact-turn".to_string();
        runtime
            .persist_turn_item(
                &thread,
                &turn,
                &TranscriptItem::UserMessage(UserMessage::text("long history".repeat(1_000))),
            )
            .await
            .unwrap();
        let outcome = runtime
            .force_compact_thread(&thread, &turn, Some("preserve the goal".into()))
            .await
            .unwrap();
        assert!(outcome.compacted);
        assert_eq!(runtime.compaction_generation(&thread), 1);
        assert_eq!(
            runtime.compaction_hysteresis_baseline(&thread),
            Some(outcome.estimated_tokens_after)
        );
        let options = runtime.compaction_options_for_turn(&thread, true);
        let grown = vec![
            crate::compaction::build_compaction_record("previous boundary".into()),
            TranscriptItem::UserMessage(UserMessage::text("new context".repeat(10_000))),
        ];
        assert!(
            crate::compaction::compaction_skip_reason(&grown, None, Some(1), &options).is_none(),
            "a prior turn boundary must permit compaction after context grows"
        );
        assert_eq!(
            runtime.compaction_generation(&thread),
            1,
            "reading options must not count as a new compaction"
        );
        assert_eq!(engine.0.lock().unwrap().len(), 2);
        let snapshot = runtime.load_thread(&thread).await.unwrap().unwrap();
        assert!(
            snapshot
                .turns
                .iter()
                .flat_map(|turn| &turn.items)
                .any(crate::compaction::is_compaction_boundary)
        );
    }
}
