use super::*;

/// Completed items are durable during sampling. Deltas remain in the caller's
/// presentation buffers until a terminal completion confirms the remainder.
#[derive(Default)]
pub(super) struct SamplingOutput {
    message_text: HashMap<String, String>,
    reasoning_text: String,
    item_ids: HashSet<String>,
}

pub(super) struct OutputContext<'a> {
    pub thread_id: &'a ThreadId,
    pub turn_id: &'a TurnId,
    pub profile: Option<&'a ModelHarnessProfile>,
    pub provider: &'a str,
    pub model: &'a str,
}

impl SamplingOutput {
    pub(super) async fn complete_item(
        &mut self,
        runtime: &Runtime,
        context: &OutputContext<'_>,
        item: serde_json::Value,
        transcript: &mut Vec<TranscriptItem>,
    ) -> anyhow::Result<()> {
        let key = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| item.to_string());
        if !self.item_ids.insert(key) {
            return Ok(());
        }
        match item.get("type").and_then(serde_json::Value::as_str) {
            Some("message") if item["role"] == "assistant" => {
                let text = text_blocks(&item, "content", "output_text");
                let phase = item
                    .get("phase")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(FINAL_ANSWER_PHASE)
                    .to_string();
                if !text.is_empty() {
                    self.message_text
                        .entry(phase.clone())
                        .or_default()
                        .push_str(&text);
                    persist(
                        runtime,
                        context,
                        TranscriptItem::AssistantMessage(AssistantMessage {
                            text,
                            phase: Some(phase),
                        }),
                        transcript,
                    )
                    .await?;
                }
            }
            Some("reasoning") => {
                let text = text_blocks(&item, "summary", "summary_text");
                if !text.is_empty() {
                    self.reasoning_text.push_str(&text);
                    persist(
                        runtime,
                        context,
                        TranscriptItem::ReasoningSummary(ReasoningSummary { text }),
                        transcript,
                    )
                    .await?;
                }
            }
            _ => {}
        }
        // Preserve opaque reasoning, completed tool calls and item ids even
        // when the HTTP stream never reaches response.completed.
        persist(
            runtime,
            context,
            TranscriptItem::ProviderMetadata(serde_json::json!({
                "output": [item],
            })),
            transcript,
        )
        .await
    }

    pub(super) async fn finish(
        &mut self,
        runtime: &Runtime,
        context: &OutputContext<'_>,
        assistant_text: &str,
        phase_messages: &[AssistantMessage],
        reasoning_text: &str,
        transcript: &mut Vec<TranscriptItem>,
    ) -> anyhow::Result<()> {
        for message in phase_messages
            .iter()
            .cloned()
            .chain((!assistant_text.is_empty()).then(|| AssistantMessage {
                text: assistant_text.to_string(),
                phase: Some(FINAL_ANSWER_PHASE.to_string()),
            }))
        {
            let phase = message.phase.as_deref().unwrap_or(FINAL_ANSWER_PHASE);
            let committed = self.message_text.entry(phase.to_string()).or_default();
            // Consume only matching prefixes. Providers without completed-item
            // events use the entire delta buffer as their terminal message.
            let text = consume_committed(&message.text, committed);
            if !text.is_empty() {
                persist(
                    runtime,
                    context,
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text,
                        phase: message.phase,
                    }),
                    transcript,
                )
                .await?;
            }
        }
        let text = consume_committed(reasoning_text, &mut self.reasoning_text);
        if !text.is_empty() {
            persist(
                runtime,
                context,
                TranscriptItem::ReasoningSummary(ReasoningSummary { text }),
                transcript,
            )
            .await?;
        }
        Ok(())
    }
}

fn consume_committed(text: &str, committed: &mut String) -> String {
    if let Some(rest) = text.strip_prefix(committed.as_str()) {
        committed.clear();
        return rest.to_string();
    }
    if committed.starts_with(text) {
        committed.drain(..text.len());
        return String::new();
    }
    text.to_string()
}

fn text_blocks(item: &serde_json::Value, field: &str, kind: &str) -> String {
    if let Some(text) = item.get(field).and_then(serde_json::Value::as_str) {
        return text.to_string();
    }
    item.get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == kind)
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect()
}

async fn persist(
    runtime: &Runtime,
    context: &OutputContext<'_>,
    item: TranscriptItem,
    transcript: &mut Vec<TranscriptItem>,
) -> anyhow::Result<()> {
    runtime
        .persist_turn_item(context.thread_id, context.turn_id, &item)
        .await?;
    if matches!(item, TranscriptItem::AssistantMessage(_)) {
        runtime
            .persist_model_profile_segment(
                context.thread_id,
                context.turn_id,
                context.profile,
                context.provider,
                context.model,
                "assistant",
            )
            .await?;
    }
    transcript.push(item);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consumes_multiple_completed_messages_without_repeating_text() {
        let mut completed = "firstsecond".to_string();
        assert_eq!(consume_committed("first", &mut completed), "");
        assert_eq!(
            consume_committed("secondunfinished", &mut completed),
            "unfinished"
        );
        assert!(completed.is_empty());
    }
}
