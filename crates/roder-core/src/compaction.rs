use roder_api::catalog::{ModelCatalogEntry, lookup_model, lookup_model_for_provider};
use roder_api::transcript::{ContextCompactionRecord, ToolResultRecord, TranscriptItem};

pub(crate) const TOOL_OUTPUT_PROTECT_TOKENS: u32 = 40_000;
pub(crate) const TOOL_OUTPUT_PRUNE_MIN_SAVINGS_TOKENS: u32 = 20_000;
pub(crate) const COMPACTION_HYSTERESIS_TOKENS: u32 = 5_000;
pub(crate) const COMPACTION_HEAD_RATIO: f32 = 0.70;
pub(crate) const COMPACTION_SUMMARY_PROMPT_MARKER: &str = "RODER_COMPACTION_STATE_SNAPSHOT";

const PRUNED_TOOL_OUTPUT_NOTICE: &str = "[tool output pruned from context to save tokens; rerun the tool with narrower scope if needed]";

#[derive(Debug, Clone, Default)]
pub(crate) struct CompactionOptions {
    /// When false, skip threshold-driven compaction if this turn already compacted.
    pub allow_repeat: bool,
    /// Manual `/compact` or emergency paths bypass threshold and hysteresis guards.
    pub force: bool,
    /// Token estimate at the last compaction trigger for hysteresis coalescing.
    pub hysteresis_baseline: Option<u32>,
    /// Optional user hint from `/compact <hint>`.
    pub preserve_hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionSkipReason {
    BelowThreshold,
    PruneSufficient,
    Hysteresis,
    AlreadyCompactedThisTurn,
}

impl CompactionSkipReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BelowThreshold => "below_threshold",
            Self::PruneSufficient => "prune_sufficient",
            Self::Hysteresis => "hysteresis",
            Self::AlreadyCompactedThisTurn => "already_compacted_this_turn",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ToolOutputPruneResult {
    pub items: Vec<TranscriptItem>,
    pub pruned_tool_count: u32,
    pub tokens_saved: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CompactionSplit {
    pub head: Vec<TranscriptItem>,
    pub tail: Vec<TranscriptItem>,
}

pub(crate) fn estimate_prompt_tokens(items: &[TranscriptItem]) -> u32 {
    let chars: usize = items
        .iter()
        .filter(|item| !matches!(item, TranscriptItem::ProviderMetadata(_)))
        .map(item_text_len)
        .sum();
    chars_to_tokens(chars)
}

pub(crate) fn trim_to_last_compaction_boundary(items: Vec<TranscriptItem>) -> Vec<TranscriptItem> {
    let Some(idx) = items
        .iter()
        .rposition(|item| matches!(item, TranscriptItem::ContextCompaction(_)))
    else {
        return items;
    };
    items[idx..].to_vec()
}

pub(crate) fn compaction_skip_reason(
    items: &[TranscriptItem],
    model_entry: Option<&ModelCatalogEntry>,
    threshold_override: Option<u32>,
    options: &CompactionOptions,
) -> Option<CompactionSkipReason> {
    if options.force || transcript_contains_context_limit_failure(items) {
        return None;
    }
    if !options.allow_repeat && transcript_already_compacted(items) {
        return Some(CompactionSkipReason::AlreadyCompactedThisTurn);
    }
    let estimated_tokens = estimate_prompt_tokens(items);
    if !meets_compaction_threshold(estimated_tokens, model_entry, threshold_override) {
        return Some(CompactionSkipReason::BelowThreshold);
    }
    if let Some(baseline) = options.hysteresis_baseline
        && estimated_tokens < baseline.saturating_add(COMPACTION_HYSTERESIS_TOKENS)
    {
        return Some(CompactionSkipReason::Hysteresis);
    }
    None
}

#[cfg(test)]
pub(crate) fn should_compact_transcript(
    items: &[TranscriptItem],
    model_entry: Option<&ModelCatalogEntry>,
    threshold_override: Option<u32>,
    options: &CompactionOptions,
) -> bool {
    compaction_skip_reason(items, model_entry, threshold_override, options).is_none()
}

pub(crate) fn prune_tool_outputs_in_transcript(items: &[TranscriptItem]) -> ToolOutputPruneResult {
    let mut protected_tokens = 0_u32;
    let mut pruned_tool_count = 0_u32;
    let mut tokens_saved = 0_u32;
    let mut pruned = Vec::with_capacity(items.len());

    for item in items.iter().rev() {
        if protected_tokens < TOOL_OUTPUT_PROTECT_TOKENS {
            protected_tokens = protected_tokens.saturating_add(item_prompt_tokens(item));
            pruned.push(item.clone());
            continue;
        }

        if let TranscriptItem::ToolResult(result) = item {
            let original_tokens = chars_to_tokens(result.result.len());
            if original_tokens <= chars_to_tokens(PRUNED_TOOL_OUTPUT_NOTICE.len()) {
                pruned.push(item.clone());
                continue;
            }
            pruned_tool_count += 1;
            tokens_saved = tokens_saved.saturating_add(
                original_tokens.saturating_sub(chars_to_tokens(PRUNED_TOOL_OUTPUT_NOTICE.len())),
            );
            pruned.push(TranscriptItem::ToolResult(ToolResultRecord {
                id: result.id.clone(),
                name: result.name.clone(),
                result: pruned_tool_output_text(&result.name, &result.result),
                display_payload: result.display_payload.clone(),
                is_error: result.is_error,
            }));
            continue;
        }

        pruned.push(item.clone());
    }

    pruned.reverse();
    ToolOutputPruneResult {
        items: pruned,
        pruned_tool_count,
        tokens_saved,
    }
}

pub(crate) fn prune_avoids_full_compaction(
    prune_result: &ToolOutputPruneResult,
    model_entry: Option<&ModelCatalogEntry>,
    threshold_override: Option<u32>,
) -> bool {
    prune_result.tokens_saved >= TOOL_OUTPUT_PRUNE_MIN_SAVINGS_TOKENS
        && !meets_compaction_threshold(
            estimate_prompt_tokens(&prune_result.items),
            model_entry,
            threshold_override,
        )
}

pub(crate) fn split_transcript_for_summarization(items: &[TranscriptItem]) -> CompactionSplit {
    let suffix = select_compaction_suffix(items);
    let suffix_start = items.len().saturating_sub(suffix.len());
    let head = items[..suffix_start].to_vec();
    if head.is_empty() {
        return CompactionSplit {
            head: items.to_vec(),
            tail: Vec::new(),
        };
    }

    CompactionSplit { head, tail: suffix }
}

pub(crate) fn head_items_for_summary_prompt(head: &[TranscriptItem]) -> Vec<TranscriptItem> {
    if head.is_empty() {
        return Vec::new();
    }
    let total_head_tokens = estimate_prompt_tokens(head);
    let summary_target_tokens =
        ((total_head_tokens as f32 * COMPACTION_HEAD_RATIO).round() as u32).max(1);
    let mut summarized_tokens = 0_u32;
    let mut split_at = head.len();
    for (index, item) in head.iter().enumerate() {
        summarized_tokens = summarized_tokens.saturating_add(item_prompt_tokens(item));
        if summarized_tokens >= summary_target_tokens {
            split_at = index + 1;
            break;
        }
    }
    head[..split_at.min(head.len())].to_vec()
}

pub(crate) fn select_compaction_suffix(items: &[TranscriptItem]) -> Vec<TranscriptItem> {
    if let Some(idx) = items
        .iter()
        .rposition(|item| matches!(item, TranscriptItem::UserMessage(_)))
    {
        items[idx..].to_vec()
    } else {
        items.last().cloned().into_iter().collect()
    }
}

pub(crate) fn summarize_transcript(items: &[TranscriptItem]) -> String {
    let mut lines = vec!["Previous transcript was compacted. Key retained facts:".to_string()];
    for item in items.iter().take(items.len().saturating_sub(1)) {
        match item {
            TranscriptItem::UserMessage(message) => {
                lines.push(format!("- user: {}", truncate(&message.text)));
            }
            TranscriptItem::AssistantMessage(message) => {
                lines.push(format!("- assistant: {}", truncate(&message.text)));
            }
            TranscriptItem::ToolResult(ToolResultRecord { name, result, .. }) => {
                let name = name.as_deref().unwrap_or("tool");
                lines.push(format!("- {name} result: {}", truncate(result)));
            }
            TranscriptItem::ContextCompaction(compaction) => {
                lines.push(format!(
                    "- prior summary: {}",
                    truncate(&compaction.summary)
                ));
            }
            TranscriptItem::ProviderMetadata(_) => {}
            _ => {}
        }
    }
    lines.join("\n")
}

pub(crate) fn build_compaction_summary_prompt(
    head: &[TranscriptItem],
    preserve_hint: Option<&str>,
) -> String {
    let mut sections = vec![
        COMPACTION_SUMMARY_PROMPT_MARKER.to_string(),
        "Summarize the conversation history below into a compact state snapshot.".to_string(),
        "Use this structure:".to_string(),
        "<state_snapshot>".to_string(),
        "goal: ...".to_string(),
        "constraints: ...".to_string(),
        "progress: ...".to_string(),
        "decisions: ...".to_string(),
        "files_and_artifacts: ...".to_string(),
        "open_questions: ...".to_string(),
        "next_steps: ...".to_string(),
        "</state_snapshot>".to_string(),
        "Keep only durable facts needed to continue work. Omit raw tool dumps.".to_string(),
    ];
    if let Some(hint) = preserve_hint.filter(|text| !text.trim().is_empty()) {
        sections.push(format!("User asked to preserve: {hint}"));
    }
    sections.push("Conversation to summarize:".to_string());
    sections.push(render_transcript_for_summary(head));
    sections.join("\n")
}

pub(crate) fn build_compaction_verify_prompt(summary: &str) -> String {
    format!(
        "{COMPACTION_SUMMARY_PROMPT_MARKER}\n\
         Review the state snapshot below. Return a tighter version only if it preserves the same facts with fewer tokens.\n\
         If it is already concise, repeat it unchanged.\n\n\
         {summary}"
    )
}

pub(crate) fn accept_llm_compaction_summary(head: &[TranscriptItem], summary: &str) -> bool {
    let head_tokens = estimate_prompt_tokens(head);
    let summary_tokens = estimate_text_tokens(summary);
    !summary.trim().is_empty() && summary_tokens < head_tokens
}

pub(crate) fn format_llm_compaction_summary(summary: &str) -> String {
    format!("Previous transcript was compacted. State snapshot:\n{summary}")
}

fn render_transcript_for_summary(items: &[TranscriptItem]) -> String {
    let mut lines = Vec::new();
    for item in items {
        match item {
            TranscriptItem::UserMessage(message) => {
                lines.push(format!(
                    "USER: {}",
                    truncate_for_summary(&message.text, 2_000)
                ));
            }
            TranscriptItem::AssistantMessage(message) => {
                lines.push(format!(
                    "ASSISTANT: {}",
                    truncate_for_summary(&message.text, 2_000)
                ));
            }
            TranscriptItem::ToolResult(ToolResultRecord { name, result, .. }) => {
                let name = name.as_deref().unwrap_or("tool");
                lines.push(format!(
                    "TOOL {name}: {}",
                    truncate_for_summary(result, 1_200)
                ));
            }
            TranscriptItem::ContextCompaction(compaction) => {
                lines.push(format!(
                    "PRIOR SUMMARY: {}",
                    truncate_for_summary(&compaction.summary, 1_200)
                ));
            }
            TranscriptItem::ProviderMetadata(_) => {}
            _ => {}
        }
    }
    lines.join("\n")
}

fn pruned_tool_output_text(name: &Option<String>, original: &str) -> String {
    let tool = name.as_deref().unwrap_or("tool");
    format!(
        "{PRUNED_TOOL_OUTPUT_NOTICE}\nTool: {tool}\nOriginal bytes: {}\nOriginal lines: {}",
        original.len(),
        original.lines().count()
    )
}

fn meets_compaction_threshold(
    estimated_tokens: u32,
    model_entry: Option<&ModelCatalogEntry>,
    threshold_override: Option<u32>,
) -> bool {
    let emergency_limit =
        model_entry.and_then(|entry| (entry.context_window > 0).then_some(entry.context_window));
    let threshold = threshold_override
        .or_else(|| model_entry.map(|entry| entry.auto_compact_token_limit))
        .unwrap_or(0);
    if model_entry.is_some_and(|entry| entry.supports_compaction) {
        emergency_limit.is_some_and(|limit| estimated_tokens >= limit)
    } else {
        threshold > 0 && estimated_tokens >= threshold
    }
}

fn transcript_already_compacted(items: &[TranscriptItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item, TranscriptItem::ContextCompaction(_)))
}

fn item_prompt_tokens(item: &TranscriptItem) -> u32 {
    chars_to_tokens(item_text_len(item))
}

fn item_text_len(item: &TranscriptItem) -> usize {
    match item {
        TranscriptItem::UserMessage(message) => message.text.len(),
        TranscriptItem::AssistantMessage(message) => message.text.len(),
        TranscriptItem::ReasoningSummary(summary) => summary.text.len(),
        TranscriptItem::ToolCall(call) => call.arguments.len() + call.name.len(),
        TranscriptItem::ToolResult(result) => result.result.len(),
        TranscriptItem::FileChange(change) => change.path.len() + change.change_type.len(),
        TranscriptItem::ContextCompaction(compaction) => compaction.summary.len(),
        TranscriptItem::Error(error) => error.message.len(),
        TranscriptItem::ProviderMetadata(_) => 0,
    }
}

fn chars_to_tokens(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

fn estimate_text_tokens(text: &str) -> u32 {
    chars_to_tokens(text.len())
}

pub(crate) fn transcript_contains_context_limit_failure(items: &[TranscriptItem]) -> bool {
    items.iter().any(|item| {
        let TranscriptItem::Error(error) = item else {
            return false;
        };
        let message = error.message.to_ascii_lowercase();
        message.contains("context window")
            || message.contains("input exceeds")
            || message.contains("response.incomplete")
            || message.contains("prompt is too long")
            || message.contains("prompt too long")
    })
}

pub(crate) fn truncate(text: &str) -> String {
    truncate_for_summary(text, 240)
}

fn truncate_for_summary(text: &str, limit: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() <= limit {
        normalized
    } else {
        let mut end = limit;
        while end > 0 && !normalized.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &normalized[..end])
    }
}

pub(crate) fn model_entry_for_compaction(
    provider: &str,
    model: &str,
) -> Option<&'static ModelCatalogEntry> {
    lookup_model_for_provider(provider, model).or_else(|| lookup_model(model))
}

pub(crate) fn build_compaction_record(summary: String) -> TranscriptItem {
    TranscriptItem::ContextCompaction(ContextCompactionRecord { summary })
}

#[cfg(test)]
mod tests {
    use roder_api::transcript::{AssistantMessage, ErrorRecord, UserMessage};

    use super::*;

    #[test]
    fn provider_metadata_is_excluded_from_prompt_token_estimates() {
        let items = vec![
            TranscriptItem::UserMessage(UserMessage::text("hello")),
            TranscriptItem::ProviderMetadata(serde_json::json!({
                "output": "x".repeat(1_000_000)
            })),
        ];
        assert_eq!(
            estimate_prompt_tokens(&items),
            chars_to_tokens("hello".len())
        );
    }

    #[test]
    fn trim_to_last_compaction_boundary_keeps_summary_and_tail() {
        let items = vec![
            TranscriptItem::UserMessage(UserMessage::text("old")),
            build_compaction_record("summary one".to_string()),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "after first".to_string(),
                phase: None,
            }),
            TranscriptItem::UserMessage(UserMessage::text("new turn")),
            build_compaction_record("summary two".to_string()),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "tool-1".to_string(),
                name: Some("read".to_string()),
                result: "tail".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];
        let trimmed = trim_to_last_compaction_boundary(items);
        assert_eq!(trimmed.len(), 2);
        assert!(matches!(
            &trimmed[0],
            TranscriptItem::ContextCompaction(record) if record.summary == "summary two"
        ));
    }

    #[test]
    fn select_compaction_suffix_keeps_active_user_turn_and_tool_work() {
        let items = vec![
            TranscriptItem::UserMessage(UserMessage::text("old context")),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "old answer".to_string(),
                phase: None,
            }),
            TranscriptItem::UserMessage(UserMessage::text("current prompt")),
            TranscriptItem::ToolCall(roder_api::transcript::ToolCallRecord {
                id: "call-1".to_string(),
                name: "read".to_string(),
                arguments: "{}".to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call-1".to_string(),
                name: Some("read".to_string()),
                result: "file contents".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];
        let suffix = select_compaction_suffix(&items);
        assert_eq!(suffix.len(), 3);
        assert!(matches!(
            &suffix[0],
            TranscriptItem::UserMessage(message) if message.text == "current prompt"
        ));
    }

    #[test]
    fn repeat_compaction_is_blocked_after_summary_exists() {
        let items = vec![
            build_compaction_record("already compacted".to_string()),
            TranscriptItem::UserMessage(UserMessage::text("continue")),
        ];
        let options = CompactionOptions {
            allow_repeat: false,
            ..CompactionOptions::default()
        };
        assert!(!should_compact_transcript(&items, None, Some(1), &options));
    }

    #[test]
    fn context_failure_forces_repeat_compaction() {
        let items = vec![
            build_compaction_record("already compacted".to_string()),
            TranscriptItem::Error(ErrorRecord {
                message: "Prompt is too long".to_string(),
            }),
        ];
        let options = CompactionOptions {
            allow_repeat: false,
            ..CompactionOptions::default()
        };
        assert!(should_compact_transcript(&items, None, Some(1), &options));
    }

    #[test]
    fn prune_masks_old_tool_outputs_outside_protect_window() {
        let items = vec![
            TranscriptItem::UserMessage(UserMessage::text("start")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "old".to_string(),
                name: Some("grep".to_string()),
                result: "x".repeat(120_000),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::UserMessage(UserMessage::text("current")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "recent-buffer".to_string(),
                name: Some("read".to_string()),
                result: "y".repeat(180_000),
                display_payload: None,
                is_error: false,
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "new".to_string(),
                name: Some("read".to_string()),
                result: "fresh output".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];
        let pruned = prune_tool_outputs_in_transcript(&items);
        assert_eq!(pruned.pruned_tool_count, 1);
        assert!(pruned.tokens_saved >= TOOL_OUTPUT_PRUNE_MIN_SAVINGS_TOKENS);
        assert!(pruned.items.iter().any(|item| matches!(
            item,
            TranscriptItem::ToolResult(result)
                if result.id == "old" && result.result.contains(PRUNED_TOOL_OUTPUT_NOTICE)
        )));
        assert!(matches!(
            pruned.items.last(),
            Some(TranscriptItem::ToolResult(result)) if result.result == "fresh output"
        ));
    }

    #[test]
    fn hysteresis_blocks_recompact_until_growth() {
        let items = vec![
            build_compaction_record("summary".to_string()),
            TranscriptItem::UserMessage(UserMessage::text("x".repeat(10_000))),
        ];
        let estimated = estimate_prompt_tokens(&items);
        let options = CompactionOptions {
            allow_repeat: true,
            hysteresis_baseline: Some(estimated),
            ..CompactionOptions::default()
        };
        assert_eq!(
            compaction_skip_reason(&items, None, Some(estimated.saturating_sub(1)), &options),
            Some(CompactionSkipReason::Hysteresis)
        );
    }

    #[test]
    fn split_transcript_for_summarization_keeps_only_active_suffix() {
        let items = vec![
            TranscriptItem::UserMessage(UserMessage::text("old".repeat(100))),
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "old answer".repeat(100),
                phase: None,
            }),
            TranscriptItem::UserMessage(UserMessage::text("current prompt")),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "tool-1".to_string(),
                name: Some("read".to_string()),
                result: "tail".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];
        let split = split_transcript_for_summarization(&items);
        assert!(!split.head.is_empty());
        assert!(split.tail.iter().any(|item| matches!(
            item,
            TranscriptItem::UserMessage(message) if message.text == "current prompt"
        )));
        assert!(split
            .head
            .iter()
            .all(|item| !matches!(item, TranscriptItem::UserMessage(message) if message.text == "current prompt")));
    }

    #[test]
    fn llm_summary_must_be_smaller_than_head() {
        let head = vec![TranscriptItem::UserMessage(UserMessage::text(
            "x".repeat(10_000),
        ))];
        assert!(!accept_llm_compaction_summary(
            &head,
            &"short".repeat(10_000)
        ));
        assert!(accept_llm_compaction_summary(&head, "short snapshot"));
    }
}
