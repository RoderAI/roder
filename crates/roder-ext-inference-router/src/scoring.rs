use roder_api::catalog::REASONING_NONE;
use roder_api::inference::{ModelSelection, ReasoningConfig};
use roder_api::inference_routing::{InferenceRoutingCandidate, InferenceRoutingContext};

use crate::config::InferenceRouterTierConfig;

#[derive(Debug, Clone)]
pub struct TierSelection {
    pub selection: ModelSelection,
    pub reasoning: Option<ReasoningConfig>,
}

pub fn selection_for_tier(
    tier: &InferenceRouterTierConfig,
    default_selection: &ModelSelection,
) -> TierSelection {
    let selection = ModelSelection {
        provider: tier
            .provider
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_selection.provider.clone()),
        model: tier
            .model
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| default_selection.model.clone()),
    };
    let reasoning = tier
        .reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|level| {
            if level == REASONING_NONE {
                ReasoningConfig::default()
            } else {
                ReasoningConfig {
                    enabled: true,
                    level: Some(level.to_string()),
                }
            }
        });

    TierSelection {
        selection,
        reasoning,
    }
}

pub fn candidate_for<'a>(
    candidates: &'a [InferenceRoutingCandidate],
    selection: &ModelSelection,
) -> Option<&'a InferenceRoutingCandidate> {
    candidates.iter().find(|candidate| {
        candidate.selection.provider == selection.provider
            && candidate.selection.model == selection.model
    })
}

pub fn candidate_rejection(
    context: &InferenceRoutingContext,
    candidate: &InferenceRoutingCandidate,
    reasoning: Option<&ReasoningConfig>,
) -> Option<String> {
    if !candidate.available {
        return Some(candidate.unavailable_reason.clone().unwrap_or_else(|| {
            format!(
                "candidate {}/{} is unavailable",
                candidate.selection.provider, candidate.selection.model
            )
        }));
    }
    if context.transcript.has_image_input && !candidate.capabilities.image_input {
        return Some(format!(
            "candidate {}/{} does not support image input",
            candidate.selection.provider, candidate.selection.model
        ));
    }
    if context.tools.requires_tool_calls && !candidate.capabilities.tool_calls {
        return Some(format!(
            "candidate {}/{} does not support tool calls",
            candidate.selection.provider, candidate.selection.model
        ));
    }
    if let (Some(estimated), Some(window)) = (
        context
            .estimated_input_tokens
            .or(context.transcript.approximate_tokens),
        candidate.model.context_window,
    ) && estimated > window
    {
        return Some(format!(
            "candidate {}/{} context window {window} is below estimated input tokens {estimated}",
            candidate.selection.provider, candidate.selection.model
        ));
    }
    if let Some(reasoning) = reasoning
        && reasoning.enabled
        && let Some(level) = reasoning.level.as_deref()
        && !candidate
            .model
            .supported_reasoning
            .iter()
            .any(|effort| effort.effort == level)
    {
        return Some(format!(
            "candidate {}/{} does not support reasoning effort {level}",
            candidate.selection.provider, candidate.selection.model
        ));
    }
    None
}
