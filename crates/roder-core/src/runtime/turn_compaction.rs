use super::*;
use roder_api::inference::CompactionProgress;
use roder_api::provider_error::{ProviderFailure, ProviderFailureKind};

// Only the runtime writes this envelope. It atomically records the boundary and
// already-durable output that would otherwise be hidden by that new boundary.
pub(crate) const COMMIT_FIELD: &str = "roder_compaction_commit";
#[cfg(test)]
#[path = "turn_compaction_tests.rs"]
mod tests;

pub(super) struct TurnCompaction {
    round_start: usize,
    candidate: Option<serde_json::Value>,
    progress: Option<CompactionProgress>,
}

impl TurnCompaction {
    pub(super) fn new(round_start: usize) -> Self {
        Self {
            round_start,
            candidate: None,
            progress: None,
        }
    }

    pub(super) fn pending(&self) -> bool {
        self.candidate.is_some()
    }

    /// Provider item completion is progress, not a successful history commit.
    pub(super) fn observe(&mut self, event: &mut InferenceEvent) -> anyhow::Result<bool> {
        match event {
            InferenceEvent::Compaction(progress) if progress.status == "completed" => {
                if let Some(item) = &progress.item {
                    self.stage(item)?;
                    self.progress = Some(progress.clone());
                    progress.status = "pending_commit".into();
                }
            }
            InferenceEvent::ProviderMetadata(metadata) => {
                if let Some(object) = metadata.as_object_mut() {
                    object.remove(COMMIT_FIELD);
                }
                if let Some(window) = metadata.get("compacted_input") {
                    let window = window
                        .as_array()
                        .filter(|items| {
                            items
                                .iter()
                                .any(crate::compaction::is_provider_compaction_item)
                        })
                        .ok_or_else(|| {
                            ProviderFailure::new(
                                ProviderFailureKind::Protocol,
                                "compacted input is missing a valid boundary",
                            )
                        })?;
                    for item in window
                        .iter()
                        .filter(|item| crate::compaction::is_provider_compaction_item(item))
                    {
                        self.stage(item)?;
                    }
                }
                if let Some(output) = metadata.get("output").and_then(serde_json::Value::as_array) {
                    for item in output
                        .iter()
                        .filter(|item| crate::compaction::is_provider_compaction_item(item))
                    {
                        self.stage(item)?;
                    }
                }
            }
            InferenceEvent::OutputItemCompleted(item)
                if crate::compaction::is_provider_compaction_item(item) =>
            {
                self.stage(item)?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn stage(&mut self, item: &serde_json::Value) -> anyhow::Result<()> {
        if !crate::compaction::is_provider_compaction_item(item)
            || (item["type"] == "compaction"
                && !item
                    .get("encrypted_content")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.is_empty()))
        {
            return Err(ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "invalid streamed compaction boundary",
            )
            .into());
        }
        if self.candidate.as_ref().is_some_and(|prior| {
            prior.get("id") != item.get("id")
                || prior.get("encrypted_content") != item.get("encrypted_content")
        }) {
            return Err(ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "streamed response contains conflicting compaction boundaries",
            )
            .into());
        }
        self.candidate = Some(item.clone());
        Ok(())
    }

    pub(super) async fn commit(
        &mut self,
        runtime: &Runtime,
        context: &OutputContext<'_>,
        metadata: &mut Option<serde_json::Value>,
        transcript: &mut Vec<TranscriptItem>,
        eager_results: Vec<ToolResultRecord>,
    ) -> anyhow::Result<bool> {
        let Some(candidate) = self.candidate.take() else {
            return Ok(false);
        };
        let mut metadata = metadata.take().unwrap_or_else(|| {
            crate::compaction::provider_metadata_from_compaction_item(candidate.clone())
        });
        crate::compaction::ensure_provider_metadata_compaction(&mut metadata, &candidate);
        let window_output: Vec<_> = metadata
            .get("compacted_input")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .skip_while(|item| !crate::compaction::is_provider_compaction_item(item))
            .skip(1)
            .cloned()
            .collect();
        // A completed response's output is the current sampling round. Keep its
        // output after the new input-history boundary, including metadata-only
        // providers that omitted the streamed compaction item from final output.
        let output = metadata["output"].as_array_mut().ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "invalid compaction response output",
            )
        })?;
        output.retain(|item| !crate::compaction::is_provider_compaction_item(item));
        output.insert(0, candidate.clone());
        for item in window_output {
            if !output.iter().any(|prior| {
                prior == &item || prior.get("id").is_some_and(|id| Some(id) == item.get("id"))
            }) {
                output.push(item);
            }
        }
        let current_output = output.clone();
        if let Some(window) = metadata
            .get_mut("compacted_input")
            .and_then(serde_json::Value::as_array_mut)
        {
            for item in current_output.into_iter().skip(1) {
                if !window.iter().any(|prior| {
                    prior == &item || prior.get("id").is_some_and(|id| Some(id) == item.get("id"))
                }) {
                    window.push(item);
                }
            }
        }
        let mut retained = transcript[self.round_start..].to_vec();
        for result in eager_results {
            let item = TranscriptItem::ToolResult(result);
            if !retained.contains(&item) {
                retained.push(item);
            }
        }
        if retained
            .iter()
            .any(crate::compaction::is_compaction_boundary)
        {
            anyhow::bail!("cannot nest compaction boundaries inside a committed replay suffix");
        }
        metadata[COMMIT_FIELD] = serde_json::json!({"version":1,"retained_transcript":retained});
        let boundary = TranscriptItem::ProviderMetadata(metadata);
        runtime
            .persist_turn_item(context.thread_id, context.turn_id, &boundary)
            .await?;
        let images = runtime.tool_images(context.provider, context.model);
        let original_item_count = transcript.len() as u64;
        let original_estimated_tokens =
            crate::compaction::estimate_prompt_tokens(transcript, images);
        *transcript = crate::compaction::trim_to_last_compaction_boundary(vec![boundary]);
        let compacted_estimated_tokens =
            crate::compaction::estimate_prompt_tokens(transcript, images);
        runtime.record_compaction_hysteresis(context.thread_id, compacted_estimated_tokens);
        runtime
            .emit(RoderEvent::ContextCompactionRecorded(
                ContextCompactionRecorded {
                    thread_id: context.thread_id.clone(),
                    turn_id: context.turn_id.clone(),
                    original_item_count,
                    original_estimated_tokens,
                    compacted_item_count: transcript.len() as u64,
                    compacted_estimated_tokens,
                    file_backed: false,
                    strategy: Some("provider_streamed".into()),
                    pruned_tool_count: None,
                    duration_ms: None,
                    timestamp: OffsetDateTime::now_utc(),
                },
            ))
            .await;
        let progress = self.progress.take().unwrap_or(CompactionProgress {
            status: "completed".into(),
            item_id: candidate
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(String::from),
            item: Some(candidate),
            tokens_before: None,
            tokens_after: None,
            duration_ms: None,
        });
        runtime
            .emit(RoderEvent::InferenceEventReceived(InferenceEventReceived {
                thread_id: context.thread_id.clone(),
                turn_id: context.turn_id.clone(),
                event: InferenceEvent::Compaction(progress),
                timestamp: OffsetDateTime::now_utc(),
            }))
            .await;
        Ok(true)
    }
}

/// Expand the runtime-owned suffix once. Quarantine invalid/nested envelopes:
/// merely retaining history is insufficient when provider replay also trims.
pub(crate) fn committed_replay(
    items: Vec<TranscriptItem>,
    boundary_index: usize,
) -> Vec<TranscriptItem> {
    let suffix = match &items[boundary_index] {
        TranscriptItem::ProviderMetadata(metadata) => metadata.get(COMMIT_FIELD),
        _ => None,
    };
    let Some(envelope) = suffix else {
        return items[boundary_index..].to_vec();
    };
    let Some(retained) = envelope
        .get("retained_transcript")
        .cloned()
        .filter(|_| envelope["version"] == 1)
        .and_then(|value| serde_json::from_value::<Vec<TranscriptItem>>(value).ok())
        .filter(|items| !items.iter().any(crate::compaction::is_compaction_boundary))
    else {
        let mut items = items;
        items.remove(boundary_index);
        return crate::compaction::trim_to_last_compaction_boundary(items);
    };
    let mut compacted = items[boundary_index..].to_vec();
    if let TranscriptItem::ProviderMetadata(metadata) = &mut compacted[0] {
        metadata.as_object_mut().unwrap().remove(COMMIT_FIELD);
    }
    compacted.splice(1..1, retained);
    compacted
}
