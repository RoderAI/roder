use std::collections::HashMap;

use futures::StreamExt;
use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceTurnContext, InstructionBundle, MessageDelta,
    ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints, RuntimeProfile,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use std::sync::Mutex;

use crate::compaction::{
    CompactionOptions, accept_llm_compaction_summary, build_compaction_summary_prompt,
    build_compaction_verify_prompt, estimate_prompt_tokens,
};
use crate::runtime::Runtime;

impl Runtime {
    pub(crate) fn record_compaction_hysteresis(&self, thread_id: &ThreadId, trigger_tokens: u32) {
        if let Ok(mut state) = self.compaction_hysteresis.lock() {
            state.insert(thread_id.clone(), trigger_tokens);
        }
    }

    pub(crate) fn compaction_hysteresis_baseline(&self, thread_id: &ThreadId) -> Option<u32> {
        self.compaction_hysteresis
            .lock()
            .ok()
            .and_then(|state| state.get(thread_id).copied())
    }

    pub(crate) fn compaction_options_for_turn(
        &self,
        thread_id: &ThreadId,
        allow_repeat: bool,
    ) -> CompactionOptions {
        CompactionOptions {
            allow_repeat,
            force: false,
            hysteresis_baseline: self.compaction_hysteresis_baseline(thread_id),
            preserve_hint: None,
        }
    }

    pub async fn force_compact_thread(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        preserve_hint: Option<String>,
    ) -> anyhow::Result<ForceCompactOutcome> {
        let cfg = self.status().await;
        let provider = cfg.default_provider.clone();
        let model = cfg.default_model.clone();
        let transcript = self.transcript_for_force_compact(thread_id).await?;
        if transcript.is_empty() {
            return Ok(ForceCompactOutcome {
                compacted: false,
                reason: Some("empty_transcript".to_string()),
                estimated_tokens_before: 0,
                estimated_tokens_after: 0,
            });
        }
        let estimated_before = estimate_prompt_tokens(&transcript);
        let compacted = self
            .compact_transcript_if_needed(
                thread_id,
                turn_id,
                &provider,
                &model,
                transcript,
                CompactionOptions {
                    allow_repeat: true,
                    force: true,
                    hysteresis_baseline: None,
                    preserve_hint: preserve_hint.filter(|text| !text.trim().is_empty()),
                },
            )
            .await?;
        let estimated_after = estimate_prompt_tokens(&compacted);
        Ok(ForceCompactOutcome {
            compacted: estimated_after < estimated_before
                || compacted
                    .iter()
                    .any(|item| matches!(item, TranscriptItem::ContextCompaction(_))),
            reason: None,
            estimated_tokens_before: estimated_before,
            estimated_tokens_after: estimated_after,
        })
    }

    async fn transcript_for_force_compact(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Vec<TranscriptItem>> {
        let Some(store) = &self.thread_store else {
            return Ok(Vec::new());
        };
        let Some(snapshot) = store.load_thread(thread_id).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for turn in snapshot.turns {
            out.extend(turn.items);
        }
        Ok(crate::compaction::trim_to_last_compaction_boundary(out))
    }

    pub(crate) async fn summarize_compaction_head(
        &self,
        provider: &str,
        model: &str,
        head: &[TranscriptItem],
        preserve_hint: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        if head.is_empty() {
            return Ok(None);
        }
        let draft = self
            .run_compaction_summary_inference(
                provider,
                model,
                build_compaction_summary_prompt(head, preserve_hint),
            )
            .await?;
        let Some(draft) = draft else {
            return Ok(None);
        };
        if !accept_llm_compaction_summary(head, &draft) {
            return Ok(None);
        }
        let verified = self
            .run_compaction_summary_inference(provider, model, build_compaction_verify_prompt(&draft))
            .await?
            .unwrap_or(draft.clone());
        if accept_llm_compaction_summary(head, &verified) {
            Ok(Some(verified))
        } else if accept_llm_compaction_summary(head, &draft) {
            Ok(Some(draft))
        } else {
            Ok(None)
        }
    }

    async fn run_compaction_summary_inference(
        &self,
        provider: &str,
        model: &str,
        prompt: String,
    ) -> anyhow::Result<Option<String>> {
        let engine = self.engine_for(provider)?;
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            instructions: InstructionBundle {
                system: Some(
                    "You compress conversation history into durable state snapshots.".to_string(),
                ),
                developer: None,
                developer_context: None,
            },
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text(prompt))],
            tools: Vec::new(),
            tool_choice: ToolChoice::None,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                profile: RuntimeProfile::Interactive,
                ..RuntimeHints::default()
            },
            metadata: serde_json::json!({ "roderCompactionSummary": true }),
        };
        let ctx = InferenceTurnContext {
            thread_id: &"compaction-summary".to_string(),
            turn_id: &"compaction-summary".to_string(),
            tool_executor: None,
        };
        let mut stream = engine.stream_turn(ctx, request).await?;
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                InferenceEvent::MessageDelta(MessageDelta { text: delta, .. }) => text.push_str(&delta),
                InferenceEvent::Failed(failure) => {
                    anyhow::bail!("compaction summary inference failed: {}", failure.message);
                }
                InferenceEvent::Completed(_) => break,
                _ => {}
            }
        }
        if text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(text.trim().to_string()))
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForceCompactOutcome {
    pub compacted: bool,
    pub reason: Option<String>,
    pub estimated_tokens_before: u32,
    pub estimated_tokens_after: u32,
}

pub(crate) fn compaction_hysteresis_state() -> Mutex<HashMap<ThreadId, u32>> {
    Mutex::new(HashMap::new())
}