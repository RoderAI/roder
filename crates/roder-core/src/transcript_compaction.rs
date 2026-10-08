use roder_api::events::*;
use roder_api::transcript::TranscriptItem;
use time::OffsetDateTime;

use crate::compaction::{
    CompactionOptions, CompactionSkipReason, build_compaction_record, compaction_skip_reason,
    estimate_prompt_tokens, head_items_for_summary_prompt, model_entry_for_compaction,
    prune_avoids_full_compaction, prune_tool_outputs_in_transcript, select_compaction_suffix,
    split_transcript_for_summarization,
};
use crate::compaction_summary::CompactionSummaryContext;
use crate::runtime::Runtime;

impl Runtime {
    pub(crate) async fn compact_transcript_if_needed(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        provider: &str,
        model: &str,
        transcript: Vec<TranscriptItem>,
        options: CompactionOptions,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let cfg = self.status().await;
        let requires_native_compaction = self.engine_for(provider)?.requires_native_compaction();
        let mut compaction_model = model_entry_for_compaction(provider, model).cloned();
        if requires_native_compaction && let Some(entry) = &mut compaction_model {
            // Explicit native compaction must fit within the context window.
            // Trigger at the normal watermark, before a context overflow.
            entry.supports_compaction = false;
        }
        let model_entry = compaction_model.as_ref();
        let threshold = cfg.auto_compact_token_limit;
        if let Some(reason) = compaction_skip_reason(&transcript, model_entry, threshold, &options)
        {
            self.emit_compaction_skipped(
                thread_id,
                turn_id,
                reason,
                estimate_prompt_tokens(&transcript),
                threshold,
                None,
            )
            .await;
            return Ok(transcript);
        }

        let mut working = transcript;
        // OpenAI must see the full window and own all compaction decisions.
        let prune_result = if requires_native_compaction {
            crate::compaction::ToolOutputPruneResult {
                items: working.clone(),
                pruned_tool_count: 0,
                tokens_saved: 0,
            }
        } else {
            prune_tool_outputs_in_transcript(&working)
        };
        let pruned_tool_count = prune_result.pruned_tool_count;
        if pruned_tool_count > 0 {
            let tokens_saved = prune_result.tokens_saved;
            working = prune_result.items;
            if !options.force
                && tokens_saved >= crate::compaction::TOOL_OUTPUT_PRUNE_MIN_SAVINGS_TOKENS
                && prune_avoids_full_compaction(
                    &crate::compaction::ToolOutputPruneResult {
                        items: working.clone(),
                        pruned_tool_count,
                        tokens_saved,
                    },
                    model_entry,
                    threshold,
                )
            {
                self.emit_compaction_skipped(
                    thread_id,
                    turn_id,
                    CompactionSkipReason::PruneSufficient,
                    estimate_prompt_tokens(&working),
                    threshold,
                    Some(pruned_tool_count),
                )
                .await;
                return Ok(working);
            }
        }

        if let Some(reason) = compaction_skip_reason(&working, model_entry, threshold, &options) {
            self.emit_compaction_skipped(
                thread_id,
                turn_id,
                reason,
                estimate_prompt_tokens(&working),
                threshold,
                Some(pruned_tool_count),
            )
            .await;
            return Ok(working);
        }

        let estimated_tokens = estimate_prompt_tokens(&working);
        let original_item_count = working.len() as u64;
        let original_estimated_tokens = estimated_tokens;
        let compaction_started_at = std::time::Instant::now();
        self.emit(RoderEvent::ContextCompactionStarted(
            ContextCompactionStarted {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                original_item_count,
                original_estimated_tokens,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;

        match self
            .compact_with_provider(
                thread_id,
                turn_id,
                provider,
                model,
                &working,
                options.preserve_hint.as_deref(),
            )
            .await?
        {
            crate::provider_compaction::NativeCompactionOutcome::Preempted(window) => {
                self.emit_compaction_skipped(
                    thread_id,
                    turn_id,
                    CompactionSkipReason::NewInput,
                    estimate_prompt_tokens(&window),
                    threshold,
                    None,
                )
                .await;
                return Ok(window);
            }
            crate::provider_compaction::NativeCompactionOutcome::Compacted(compacted) => {
                self.record_compaction_hysteresis(thread_id, estimate_prompt_tokens(&compacted));
                self.emit(RoderEvent::ContextCompactionRecorded(
                    ContextCompactionRecorded {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        original_item_count,
                        original_estimated_tokens,
                        compacted_item_count: compacted.len() as u64,
                        compacted_estimated_tokens: estimate_prompt_tokens(&compacted),
                        file_backed: false,
                        strategy: Some("provider_native".into()),
                        pruned_tool_count: Some(pruned_tool_count),
                        duration_ms: u64::try_from(compaction_started_at.elapsed().as_millis())
                            .ok(),
                        timestamp: OffsetDateTime::now_utc(),
                    },
                ))
                .await;
                return Ok(compacted);
            }
            crate::provider_compaction::NativeCompactionOutcome::Unsupported => {}
        }

        let split = split_transcript_for_summarization(&working);
        let suffix = if split.tail.is_empty() {
            select_compaction_suffix(&working)
        } else {
            split.tail
        };
        let summary_head = head_items_for_summary_prompt(&split.head);
        let (summary, strategy) = self
            .build_compaction_summary(
                CompactionSummaryContext {
                    thread_id,
                    turn_id,
                    provider,
                    model,
                },
                &summary_head,
                &working,
                options.preserve_hint.as_deref(),
                cfg.file_backed_dynamic_context,
            )
            .await?;
        let compaction = build_compaction_record(summary);
        self.persist_turn_item(thread_id, turn_id, &compaction)
            .await?;
        let mut compacted = vec![compaction];
        compacted.extend(suffix);
        self.record_compaction_hysteresis(thread_id, estimate_prompt_tokens(&compacted));
        let compacted_estimated_tokens = estimate_prompt_tokens(&compacted);
        let duration_ms = u64::try_from(compaction_started_at.elapsed().as_millis()).ok();
        self.emit(RoderEvent::ContextCompactionRecorded(
            ContextCompactionRecorded {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                original_item_count,
                original_estimated_tokens,
                compacted_item_count: compacted.len() as u64,
                compacted_estimated_tokens,
                file_backed: cfg.file_backed_dynamic_context,
                strategy: Some(strategy),
                pruned_tool_count: Some(pruned_tool_count),
                duration_ms,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
        Ok(compacted)
    }

    async fn emit_compaction_skipped(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        reason: CompactionSkipReason,
        estimated_tokens: u32,
        threshold: Option<u32>,
        pruned_tool_count: Option<u32>,
    ) {
        if matches!(
            reason,
            CompactionSkipReason::BelowThreshold | CompactionSkipReason::AlreadyCompactedThisTurn
        ) {
            return;
        }
        self.emit(RoderEvent::ContextCompactionSkipped(
            ContextCompactionSkipped {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                reason: reason.as_str().to_string(),
                estimated_tokens,
                threshold,
                pruned_tool_count,
                timestamp: OffsetDateTime::now_utc(),
            },
        ))
        .await;
    }
}
