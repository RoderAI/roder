use roder_api::extension::ExtensionRegistry;
use roder_api::inference::{
    InferenceProviderContext, ModelSelection, ReasoningConfig, RuntimeProfile, SpeedPolicyPhase,
};
use roder_api::inference_routing::{
    InferenceRoutingCandidate, InferenceRoutingContext, InferenceRoutingDecision,
    InferenceRoutingOutcome, InferenceRoutingSignal, InferenceRoutingToolSummary,
    InferenceRoutingTranscriptSummary,
};
use roder_api::tools::ToolSpec;
use roder_api::transcript::TranscriptItem;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeInferenceRouterConfig {
    pub enabled: bool,
    pub router_id: Option<String>,
}

impl RuntimeInferenceRouterConfig {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.enabled && self.router_id.is_some()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InferenceRoutingRequest<'a> {
    pub(crate) thread_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) round_index: u32,
    pub(crate) runtime_profile: RuntimeProfile,
    pub(crate) phase: SpeedPolicyPhase,
    pub(crate) profile: Option<&'a str>,
    pub(crate) default_selection: ModelSelection,
    pub(crate) transcript: &'a [TranscriptItem],
    pub(crate) tools: &'a [ToolSpec],
    pub(crate) candidates: Option<&'a [InferenceRoutingCandidate]>,
    pub(crate) prior_failures: u32,
    pub(crate) prior_escalations: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct InferenceRoutingSelection {
    pub(crate) selection: ModelSelection,
    pub(crate) reasoning: Option<ReasoningConfig>,
    pub(crate) decision: Option<InferenceRoutingDecision>,
}

impl InferenceRoutingSelection {
    fn default(selection: ModelSelection) -> Self {
        Self {
            selection,
            reasoning: None,
            decision: None,
        }
    }
}

pub(crate) async fn route_inference_selection(
    registry: &ExtensionRegistry,
    config: &RuntimeInferenceRouterConfig,
    request: InferenceRoutingRequest<'_>,
) -> InferenceRoutingSelection {
    if !config.is_active() {
        return InferenceRoutingSelection::default(request.default_selection);
    }

    let router_id = config.router_id.as_deref().unwrap_or_default();
    let Some(router) = registry.inference_router(router_id) else {
        return fallback_selection(
            request.default_selection,
            router_id,
            format!("inference router {router_id:?} is not registered"),
        );
    };

    let collected_candidates;
    let candidates = if let Some(candidates) = request.candidates {
        candidates
    } else {
        collected_candidates = collect_inference_routing_candidates(registry).await;
        &collected_candidates
    };
    let context = InferenceRoutingContext {
        thread_id: request.thread_id.to_string(),
        turn_id: request.turn_id.to_string(),
        round_index: request.round_index,
        runtime_profile: request.runtime_profile,
        default_selection: request.default_selection.clone(),
        requested_selection: None,
        phase: Some(request.phase),
        transcript: transcript_summary(request.transcript),
        tools: tool_summary(request.tools),
        candidates: candidates.to_vec(),
        signals: initial_signals(request.phase, request.profile),
        prior_failures: request.prior_failures,
        prior_escalations: request.prior_escalations,
        estimated_input_tokens: approximate_transcript_tokens(request.transcript),
    };

    let decision = match router.route(context).await {
        Ok(decision) => decision,
        Err(err) => {
            return fallback_selection(
                request.default_selection,
                router_id,
                format!("inference router {router_id:?} failed: {err}"),
            );
        }
    };

    apply_decision(
        request.default_selection,
        request.transcript,
        candidates,
        decision,
    )
}

fn apply_decision(
    default_selection: ModelSelection,
    transcript: &[TranscriptItem],
    candidates: &[InferenceRoutingCandidate],
    decision: InferenceRoutingDecision,
) -> InferenceRoutingSelection {
    match decision.outcome {
        InferenceRoutingOutcome::Selected | InferenceRoutingOutcome::Escalated => {
            let Some(selected) = decision.selected.clone() else {
                return fallback_selection(
                    default_selection,
                    decision.router_id,
                    "router selected no provider/model",
                );
            };
            let Some(candidate) = candidate_for(candidates, &selected) else {
                return fallback_selection(
                    default_selection,
                    decision.router_id,
                    format!(
                        "router selected unavailable provider/model {}/{}",
                        selected.provider, selected.model
                    ),
                );
            };
            if let Some(reason) = invalid_candidate_reason(candidate, transcript) {
                return fallback_selection(default_selection, decision.router_id, reason);
            }
            let reasoning = decision
                .reasoning
                .as_ref()
                .filter(|reasoning| reasoning_supported(candidate, reasoning))
                .cloned();
            InferenceRoutingSelection {
                selection: selected,
                reasoning,
                decision: Some(decision),
            }
        }
        InferenceRoutingOutcome::Abstained | InferenceRoutingOutcome::Fallback => {
            InferenceRoutingSelection {
                selection: default_selection,
                reasoning: None,
                decision: Some(decision),
            }
        }
    }
}

fn fallback_selection(
    default_selection: ModelSelection,
    router_id: impl Into<String>,
    reason: impl Into<String>,
) -> InferenceRoutingSelection {
    InferenceRoutingSelection {
        selection: default_selection,
        reasoning: None,
        decision: Some(InferenceRoutingDecision::fallback(router_id, reason)),
    }
}

pub async fn collect_inference_routing_candidates(
    registry: &ExtensionRegistry,
) -> Vec<InferenceRoutingCandidate> {
    let mut candidates = Vec::new();
    for engine in &registry.inference_engines {
        let provider_id = engine.id();
        let provider = engine.metadata();
        let capabilities = engine.capabilities();
        let auth_available = provider.auth_configured.unwrap_or(true);
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: &provider_id,
            })
            .await
            .unwrap_or_default();
        for model in models {
            let selection = ModelSelection {
                provider: provider_id.clone(),
                model: model.id.clone(),
            };
            if auth_available {
                candidates.push(InferenceRoutingCandidate::available(
                    selection,
                    provider.clone(),
                    model,
                    capabilities.clone(),
                ));
            } else {
                candidates.push(InferenceRoutingCandidate::unavailable(
                    selection,
                    provider.clone(),
                    model,
                    capabilities.clone(),
                    "provider authentication is not configured",
                ));
            }
        }
    }
    candidates
}

fn transcript_summary(transcript: &[TranscriptItem]) -> InferenceRoutingTranscriptSummary {
    InferenceRoutingTranscriptSummary {
        item_count: transcript.len() as u32,
        user_message_count: transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::UserMessage(_)))
            .count() as u32,
        assistant_message_count: transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::AssistantMessage(_)))
            .count() as u32,
        tool_result_count: transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::ToolResult(_)))
            .count() as u32,
        has_image_input: transcript_has_images(transcript),
        latest_user_message_preview: latest_user_message_preview(transcript),
        recent_tool_names: recent_tool_names(transcript),
        approximate_tokens: approximate_transcript_tokens(transcript),
    }
}

fn tool_summary(tools: &[ToolSpec]) -> InferenceRoutingToolSummary {
    InferenceRoutingToolSummary {
        available_count: tools.len() as u32,
        has_file_tools: tools.iter().any(|tool| {
            tool.name.contains("file") || tool.name.contains("read") || tool.name.contains("write")
        }),
        has_shell_tools: tools
            .iter()
            .any(|tool| tool.name.contains("shell") || tool.name.contains("command")),
        has_network_tools: tools
            .iter()
            .any(|tool| tool.name.contains("web") || tool.name.contains("search")),
        requires_tool_calls: !tools.is_empty(),
    }
}

fn initial_signals(phase: SpeedPolicyPhase, profile: Option<&str>) -> Vec<InferenceRoutingSignal> {
    let mut signals = vec![InferenceRoutingSignal::new("phase", phase.as_str())];
    if let Some(profile) = profile {
        signals.push(InferenceRoutingSignal::new("profile", profile));
    }
    signals
}

fn candidate_for<'a>(
    candidates: &'a [InferenceRoutingCandidate],
    selection: &ModelSelection,
) -> Option<&'a InferenceRoutingCandidate> {
    candidates.iter().find(|candidate| {
        candidate.selection.provider == selection.provider
            && candidate.selection.model == selection.model
    })
}

fn invalid_candidate_reason(
    candidate: &InferenceRoutingCandidate,
    transcript: &[TranscriptItem],
) -> Option<String> {
    inference_routing_candidate_unavailable_reason(candidate, transcript_has_images(transcript))
}

pub fn inference_routing_selection_unavailable_reason(
    candidates: &[InferenceRoutingCandidate],
    selection: &ModelSelection,
    has_image_input: bool,
) -> Option<String> {
    let Some(candidate) = candidate_for(candidates, selection) else {
        return Some(format!(
            "router selected unavailable provider/model {}/{}",
            selection.provider, selection.model
        ));
    };
    inference_routing_candidate_unavailable_reason(candidate, has_image_input)
}

pub fn inference_routing_candidate_unavailable_reason(
    candidate: &InferenceRoutingCandidate,
    has_image_input: bool,
) -> Option<String> {
    if !candidate.available {
        return Some(candidate.unavailable_reason.clone().unwrap_or_else(|| {
            format!(
                "router selected unavailable provider/model {}/{}",
                candidate.selection.provider, candidate.selection.model
            )
        }));
    }
    if has_image_input && !candidate.capabilities.image_input {
        return Some(format!(
            "router selected {}/{} but it does not support image input",
            candidate.selection.provider, candidate.selection.model
        ));
    }
    None
}

fn reasoning_supported(candidate: &InferenceRoutingCandidate, reasoning: &ReasoningConfig) -> bool {
    if !reasoning.enabled {
        return true;
    }
    let Some(level) = reasoning.level.as_deref() else {
        return true;
    };
    candidate
        .model
        .supported_reasoning
        .iter()
        .any(|option| option.effort == level)
}

fn transcript_has_images(transcript: &[TranscriptItem]) -> bool {
    transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::UserMessage(message) if !message.images.is_empty()
        )
    })
}

fn approximate_transcript_tokens(transcript: &[TranscriptItem]) -> Option<u32> {
    let bytes = transcript
        .iter()
        .map(transcript_item_text_len)
        .sum::<usize>();
    Some((bytes / 4).max(transcript.len()) as u32)
}

pub(crate) fn transcript_failure_count(transcript: &[TranscriptItem]) -> u32 {
    transcript
        .iter()
        .filter(|item| match item {
            TranscriptItem::ToolResult(result) => result.is_error,
            TranscriptItem::Error(_) => true,
            _ => false,
        })
        .count() as u32
}

pub(crate) fn transcript_failure_count_since(
    transcript: &[TranscriptItem],
    start_index: usize,
) -> u32 {
    transcript_failure_count(
        transcript
            .get(start_index.min(transcript.len())..)
            .unwrap_or(&[]),
    )
}

fn latest_user_message_preview(transcript: &[TranscriptItem]) -> Option<String> {
    transcript.iter().rev().find_map(|item| {
        let TranscriptItem::UserMessage(message) = item else {
            return None;
        };
        let text = message.text.trim();
        (!text.is_empty()).then(|| truncate_chars(text, 600))
    })
}

fn recent_tool_names(transcript: &[TranscriptItem]) -> Vec<String> {
    transcript
        .iter()
        .rev()
        .filter_map(|item| match item {
            TranscriptItem::ToolCall(call) => Some(call.name.clone()),
            TranscriptItem::ToolResult(result) => result.name.clone(),
            _ => None,
        })
        .take(12)
        .collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn transcript_item_text_len(item: &TranscriptItem) -> usize {
    match item {
        TranscriptItem::UserMessage(message) => message.text.len(),
        TranscriptItem::AssistantMessage(message) => message.text.len(),
        TranscriptItem::ReasoningSummary(summary) => summary.text.len(),
        TranscriptItem::ToolCall(call) => call.arguments.len(),
        TranscriptItem::ToolResult(result) => result.result.len(),
        TranscriptItem::FileChange(change) => change.path.len() + change.change_type.len(),
        TranscriptItem::ContextCompaction(compaction) => compaction.summary.len(),
        TranscriptItem::Error(error) => error.message.len(),
        TranscriptItem::ProviderMetadata(metadata) => metadata.to_string().len(),
    }
}

#[cfg(test)]
mod tests {
    use roder_api::transcript::{ToolResultRecord, UserMessage};

    use super::*;

    #[test]
    fn transcript_failure_count_since_ignores_prior_history() {
        let transcript = vec![
            TranscriptItem::ToolResult(tool_result("old", true)),
            TranscriptItem::UserMessage(UserMessage::text("current turn")),
            TranscriptItem::ToolResult(tool_result("ok", false)),
            TranscriptItem::ToolResult(tool_result("new", true)),
        ];

        assert_eq!(transcript_failure_count_since(&transcript, 1), 1);
        assert_eq!(transcript_failure_count_since(&transcript, 0), 2);
    }

    fn tool_result(id: &str, is_error: bool) -> ToolResultRecord {
        ToolResultRecord {
            id: id.to_string(),
            name: Some("test".to_string()),
            result: if is_error { "error" } else { "ok" }.to_string(),
            display_payload: None,
            is_error,
        }
    }
}
