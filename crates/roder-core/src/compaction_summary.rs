use crate::compaction::{format_llm_compaction_summary, summarize_transcript};
use crate::runtime::Runtime;
use roder_api::artifacts::{ContextArtifactKind, CreateArtifactRequest, format_artifact_reference};
use roder_api::events::{ContextArtifactCreated, RoderEvent, ThreadId, TurnId};
use roder_api::transcript::TranscriptItem;
use time::OffsetDateTime;

pub(crate) struct CompactionSummaryContext<'a> {
    pub thread_id: &'a ThreadId,
    pub turn_id: &'a TurnId,
    pub provider: &'a str,
    pub model: &'a str,
}
impl Runtime {
    pub(crate) async fn build_compaction_summary(
        &self,
        context: CompactionSummaryContext<'_>,
        head: &[TranscriptItem],
        full_transcript: &[TranscriptItem],
        preserve_hint: Option<&str>,
        file_backed: bool,
    ) -> anyhow::Result<(String, String)> {
        let CompactionSummaryContext {
            thread_id,
            turn_id,
            provider,
            model,
        } = context;
        let mut used_llm = false;
        let mut summary = if head.is_empty() {
            summarize_transcript(full_transcript)
        } else if let Some(llm_summary) = self
            .summarize_compaction_head(thread_id, turn_id, provider, model, head, preserve_hint)
            .await?
        {
            used_llm = true;
            format_llm_compaction_summary(&llm_summary)
        } else {
            summarize_transcript(full_transcript)
        };
        let mut strategy = if used_llm {
            "llm".to_string()
        } else {
            "deterministic".to_string()
        };

        if file_backed {
            let history_json = serde_json::to_vec_pretty(full_transcript)?;
            let artifact = self.context_artifacts().create(CreateArtifactRequest {
                kind: ContextArtifactKind::ChatHistory,
                thread_id,
                turn_id,
                source_tool_id: None,
                label: Some("pre-compaction transcript"),
                bytes: &history_json,
            })?;
            self.emit(RoderEvent::ContextArtifactCreated(ContextArtifactCreated {
                thread_id: thread_id.clone(),
                turn_id: turn_id.clone(),
                artifact: artifact.clone(),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
            let reference = format_artifact_reference(&artifact, "pre-compaction transcript");
            summary = format!("{summary}\n\n{reference}");
            if strategy == "llm" {
                strategy = "llm_file_backed".to_string();
            } else {
                strategy = "deterministic_file_backed".to_string();
            }
        }
        Ok((summary, strategy))
    }
}
