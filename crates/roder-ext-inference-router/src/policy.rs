use roder_api::inference::ModelSelection;
use roder_api::inference_routing::{
    InferenceRoutingContext, InferenceRoutingCostDelta, InferenceRoutingCostEstimate,
    InferenceRoutingDecision,
};
use serde_json::json;

use crate::config::{
    InferenceRouterPriceConfig, InferenceRouterProfileConfig, LocalInferenceRouterConfig,
};
use crate::extension::LOCAL_INFERENCE_ROUTER_ID;
use crate::profiler::{ProfiledTurn, profile_context};
use crate::scoring::{candidate_for, candidate_rejection, selection_for_tier};

struct ActiveProfile<'a> {
    name: Option<&'a str>,
    profile: Option<&'a InferenceRouterProfileConfig>,
}

struct TierChoice {
    tier: String,
    reason: String,
    escalated: bool,
    confidence: f64,
}

pub fn route(
    config: &LocalInferenceRouterConfig,
    context: InferenceRoutingContext,
) -> InferenceRoutingDecision {
    if !config.enabled {
        return InferenceRoutingDecision::abstain(
            LOCAL_INFERENCE_ROUTER_ID,
            "local inference router config is disabled",
        );
    }

    let active = match active_profile(config) {
        Ok(active) => active,
        Err(reason) => {
            return InferenceRoutingDecision::abstain(LOCAL_INFERENCE_ROUTER_ID, reason);
        }
    };
    let classifier_prompt = active
        .profile
        .and_then(|profile| profile.classifier_prompt.as_deref());
    let profiled = profile_context(&context, classifier_prompt);
    let Some(choice) = choose_tier(config, &active, &profiled) else {
        return abstain_with_profile(
            config,
            &active,
            &profiled,
            "no inference router tier is configured for this profile",
        );
    };
    let Some(tier) = config.tiers.get(&choice.tier) else {
        return abstain_with_profile(
            config,
            &active,
            &profiled,
            format!(
                "configured inference router tier {:?} is missing",
                choice.tier
            ),
        );
    };

    let tier_selection = selection_for_tier(tier, &context.default_selection);
    let Some(candidate) = candidate_for(&context.candidates, &tier_selection.selection) else {
        return abstain_with_profile(
            config,
            &active,
            &profiled,
            format!(
                "configured inference router tier {:?} selected unavailable provider/model {}/{}",
                choice.tier, tier_selection.selection.provider, tier_selection.selection.model
            ),
        );
    };
    if let Some(reason) =
        candidate_rejection(&context, candidate, tier_selection.reasoning.as_ref())
    {
        return abstain_with_profile(config, &active, &profiled, reason);
    }

    let mut decision = if choice.escalated {
        InferenceRoutingDecision::escalated(
            LOCAL_INFERENCE_ROUTER_ID,
            tier_selection.selection,
            choice.reason,
        )
    } else {
        InferenceRoutingDecision::selected(
            LOCAL_INFERENCE_ROUTER_ID,
            tier_selection.selection,
            choice.reason,
        )
    };
    decision.reasoning = tier_selection.reasoning;
    decision.confidence = Some(choice.confidence);
    decision.metadata = route_metadata(config, &active, &profiled, Some(&choice.tier));
    decision.matched_signals = profiled.signals;
    decision.baseline = baseline_selection(config, &context.default_selection);
    decision.cost_delta = cost_delta(
        config,
        decision.selected.as_ref(),
        decision.baseline.as_ref(),
        context
            .estimated_input_tokens
            .or(context.transcript.approximate_tokens),
    );
    decision
}

fn active_profile(config: &LocalInferenceRouterConfig) -> Result<ActiveProfile<'_>, String> {
    if let Some(name) = config
        .profile
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let Some(profile) = config.profiles.get(name) else {
            return Err(format!(
                "inference router profile {name:?} is not configured"
            ));
        };
        return Ok(ActiveProfile {
            name: Some(name),
            profile: Some(profile),
        });
    }
    if config.profiles.len() == 1 {
        let (name, profile) = config
            .profiles
            .iter()
            .next()
            .expect("profile length checked");
        return Ok(ActiveProfile {
            name: Some(name.as_str()),
            profile: Some(profile),
        });
    }
    Ok(ActiveProfile {
        name: None,
        profile: None,
    })
}

fn choose_tier(
    config: &LocalInferenceRouterConfig,
    active: &ActiveProfile<'_>,
    profiled: &ProfiledTurn,
) -> Option<TierChoice> {
    let profile = active.profile;
    if profiled.recovery {
        return tier_choice(
            config,
            [
                risk_floor_tier(profile, profiled),
                profile.and_then(|profile| profile.strong_tier.as_deref()),
                profile.and_then(|profile| profile.risk_floor_tier.as_deref()),
                profile.and_then(|profile| profile.default_tier.as_deref()),
            ],
            "recovery or prior failure signal",
            true,
            0.84,
        );
    }
    if profiled.high_risk {
        return tier_choice(
            config,
            [
                risk_floor_tier(profile, profiled),
                profile.and_then(|profile| profile.risk_floor_tier.as_deref()),
                profile.and_then(|profile| profile.strong_tier.as_deref()),
                profile.and_then(|profile| profile.default_tier.as_deref()),
            ],
            "risk floor signal",
            true,
            0.82,
        );
    }

    let objective = objective(config, active);
    let quality_biased = objective
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("quality") || value.contains("accuracy") || value.contains("safety")
        })
        .unwrap_or(false);
    if profiled.routine && !quality_biased {
        return tier_choice(
            config,
            [
                profile.and_then(|profile| profile.simple_tier.as_deref()),
                profile.and_then(|profile| profile.default_tier.as_deref()),
                profile.and_then(|profile| profile.standard_tier.as_deref()),
            ],
            "routine local-profiler signal",
            false,
            0.76,
        );
    }

    tier_choice(
        config,
        [
            profile.and_then(|profile| profile.standard_tier.as_deref()),
            profile.and_then(|profile| profile.default_tier.as_deref()),
            conventional_tier(config, "standard"),
            conventional_tier(config, "default"),
        ],
        "standard local-profiler signal",
        false,
        0.68,
    )
}

fn risk_floor_tier<'a>(
    profile: Option<&'a InferenceRouterProfileConfig>,
    profiled: &ProfiledTurn,
) -> Option<&'a str> {
    let profile = profile?;
    for risk in &profiled.risks {
        if let Some(tier) = profile.risk_floors.get(risk) {
            return Some(tier.as_str());
        }
    }
    None
}

fn tier_choice<'a>(
    config: &LocalInferenceRouterConfig,
    configured: impl IntoIterator<Item = Option<&'a str>>,
    reason: &str,
    escalated: bool,
    confidence: f64,
) -> Option<TierChoice> {
    for name in configured {
        if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
            return Some(TierChoice {
                tier: name.to_string(),
                reason: reason.to_string(),
                escalated,
                confidence,
            });
        }
    }
    for fallback in ["simple", "standard", "default", "strong"] {
        if config.tiers.contains_key(fallback) {
            return Some(TierChoice {
                tier: fallback.to_string(),
                reason: reason.to_string(),
                escalated,
                confidence,
            });
        }
    }
    None
}

fn conventional_tier<'a>(
    config: &'a LocalInferenceRouterConfig,
    name: &'static str,
) -> Option<&'a str> {
    config.tiers.contains_key(name).then_some(name)
}

fn baseline_selection(
    config: &LocalInferenceRouterConfig,
    default_selection: &ModelSelection,
) -> Option<ModelSelection> {
    match (&config.baseline_provider, &config.baseline_model) {
        (Some(provider), Some(model)) => Some(ModelSelection {
            provider: provider.clone(),
            model: model.clone(),
        }),
        _ => Some(default_selection.clone()),
    }
}

fn cost_delta(
    config: &LocalInferenceRouterConfig,
    selected: Option<&ModelSelection>,
    baseline: Option<&ModelSelection>,
    estimated_input_tokens: Option<u32>,
) -> Option<InferenceRoutingCostDelta> {
    let selected = selected?;
    let baseline = baseline?;
    let input_tokens = estimated_input_tokens?;
    let selected_price = price_for(config, selected)?;
    let baseline_price = price_for(config, baseline)?;
    let selected_estimate = cost_estimate(selected, selected_price, input_tokens);
    let baseline_estimate = cost_estimate(baseline, baseline_price, input_tokens);
    Some(InferenceRoutingCostDelta {
        estimated_savings_usd: baseline_estimate.total_cost_usd - selected_estimate.total_cost_usd,
        selected_estimate,
        baseline_estimate,
        classifier_overhead_usd: None,
    })
}

fn price_for<'a>(
    config: &'a LocalInferenceRouterConfig,
    selection: &ModelSelection,
) -> Option<&'a InferenceRouterPriceConfig> {
    config
        .prices
        .get(&format!("{}/{}", selection.provider, selection.model))
        .or_else(|| config.prices.get(&selection.model))
}

fn cost_estimate(
    selection: &ModelSelection,
    price: &InferenceRouterPriceConfig,
    input_tokens: u32,
) -> InferenceRoutingCostEstimate {
    let prompt_cost_usd = price
        .input_per_million
        .map(|price| (input_tokens as f64 / 1_000_000.0) * price)
        .unwrap_or(0.0);
    InferenceRoutingCostEstimate {
        selection: selection.clone(),
        prompt_cost_usd,
        completion_cost_usd: 0.0,
        total_cost_usd: prompt_cost_usd,
        price_source: "inference_router.extension.prices".to_string(),
        usage_source: "estimated_input_tokens".to_string(),
        incomplete: true,
    }
}

fn objective<'a>(
    config: &'a LocalInferenceRouterConfig,
    active: &ActiveProfile<'a>,
) -> Option<&'a str> {
    active
        .profile
        .and_then(|profile| profile.objective.as_deref())
        .or(config.objective.as_deref())
}

fn abstain_with_profile(
    config: &LocalInferenceRouterConfig,
    active: &ActiveProfile<'_>,
    profiled: &ProfiledTurn,
    reason: impl Into<String>,
) -> InferenceRoutingDecision {
    let mut decision = InferenceRoutingDecision::abstain(LOCAL_INFERENCE_ROUTER_ID, reason);
    decision.confidence = Some(0.5);
    decision.matched_signals = profiled.signals.clone();
    decision.metadata = route_metadata(config, active, profiled, None);
    decision
}

fn route_metadata(
    config: &LocalInferenceRouterConfig,
    active: &ActiveProfile<'_>,
    profiled: &ProfiledTurn,
    tier: Option<&str>,
) -> serde_json::Value {
    json!({
        "tier": tier,
        "profile": active.name,
        "objective": objective(config, active),
        "routine": profiled.routine,
        "highRisk": profiled.high_risk,
        "recovery": profiled.recovery,
        "risks": profiled.risks.clone(),
        "intents": profiled.intents.clone(),
        "classifierComparison": {
            "enabled": config.classifier_comparison.enabled,
            "label": config.classifier_comparison.label.as_deref(),
            "reserved": true
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{InferenceRouterProfileConfig, InferenceRouterTierConfig};
    use roder_api::inference::{
        InferenceCapabilities, InferenceProviderMetadata, ModelDescriptor,
        ReasoningEffortDescriptor, RuntimeProfile, SpeedPolicyPhase,
    };
    use roder_api::inference_routing::{
        InferenceRoutingCandidate, InferenceRoutingOutcome, InferenceRoutingToolSummary,
        InferenceRoutingTranscriptSummary,
    };

    use super::*;

    #[test]
    fn route_uses_simple_tier_for_routine_intent() {
        let config = test_config();
        let decision = route(&config, test_context("please fix a typo", false, 0));

        assert_eq!(decision.outcome, InferenceRoutingOutcome::Selected);
        assert_eq!(decision.selected.unwrap().model, "fast");
        assert_eq!(decision.reasoning.unwrap().level.as_deref(), Some("low"));
        assert!(
            decision
                .cost_delta
                .as_ref()
                .is_some_and(|delta| delta.estimated_savings_usd > 0.0)
        );
    }

    #[test]
    fn route_escalates_risk_floor() {
        let config = test_config();
        let decision = route(
            &config,
            test_context("review auth token permission bug", false, 0),
        );

        assert_eq!(decision.outcome, InferenceRoutingOutcome::Escalated);
        assert_eq!(decision.selected.unwrap().model, "strong");
        assert_eq!(decision.metadata["tier"], "strong");
    }

    #[test]
    fn route_abstains_when_tier_candidate_is_incompatible() {
        let config = test_config();
        let mut context = test_context("fix a typo", false, 0);
        context.candidates[0].model.context_window = Some(10);
        let decision = route(&config, context);

        assert_eq!(decision.outcome, InferenceRoutingOutcome::Abstained);
        assert!(decision.reason.contains("context window"));
    }

    fn test_config() -> LocalInferenceRouterConfig {
        LocalInferenceRouterConfig {
            enabled: true,
            profile: Some("coding".to_string()),
            objective: Some("cost".to_string()),
            tiers: HashMap::from([
                (
                    "simple".to_string(),
                    InferenceRouterTierConfig {
                        provider: Some("mock".to_string()),
                        model: Some("fast".to_string()),
                        reasoning: Some("low".to_string()),
                    },
                ),
                (
                    "strong".to_string(),
                    InferenceRouterTierConfig {
                        provider: Some("mock".to_string()),
                        model: Some("strong".to_string()),
                        reasoning: Some("high".to_string()),
                    },
                ),
            ]),
            profiles: HashMap::from([(
                "coding".to_string(),
                InferenceRouterProfileConfig {
                    default_tier: Some("simple".to_string()),
                    simple_tier: Some("simple".to_string()),
                    strong_tier: Some("strong".to_string()),
                    risk_floor_tier: Some("strong".to_string()),
                    risk_floors: HashMap::from([("security".to_string(), "strong".to_string())]),
                    ..InferenceRouterProfileConfig::default()
                },
            )]),
            prices: HashMap::from([
                (
                    "mock/fast".to_string(),
                    InferenceRouterPriceConfig {
                        input_per_million: Some(0.10),
                        output_per_million: Some(0.40),
                        cached_input_per_million: None,
                    },
                ),
                (
                    "mock/strong".to_string(),
                    InferenceRouterPriceConfig {
                        input_per_million: Some(2.00),
                        output_per_million: Some(8.00),
                        cached_input_per_million: None,
                    },
                ),
            ]),
            ..LocalInferenceRouterConfig::default()
        }
    }

    fn test_context(text: &str, image: bool, failures: u32) -> InferenceRoutingContext {
        InferenceRoutingContext {
            thread_id: "thread".to_string(),
            turn_id: "turn".to_string(),
            round_index: 0,
            runtime_profile: RuntimeProfile::Interactive,
            default_selection: ModelSelection {
                provider: "mock".to_string(),
                model: "strong".to_string(),
            },
            requested_selection: None,
            phase: Some(SpeedPolicyPhase::Execution),
            transcript: InferenceRoutingTranscriptSummary {
                has_image_input: image,
                latest_user_message_preview: Some(text.to_string()),
                approximate_tokens: Some(100),
                ..InferenceRoutingTranscriptSummary::default()
            },
            tools: InferenceRoutingToolSummary::default(),
            candidates: vec![
                candidate("fast", false, "low"),
                candidate("strong", true, "high"),
            ],
            signals: Vec::new(),
            prior_failures: failures,
            prior_escalations: 0,
            estimated_input_tokens: Some(100),
        }
    }

    fn candidate(model: &str, image_input: bool, reasoning: &str) -> InferenceRoutingCandidate {
        InferenceRoutingCandidate::available(
            ModelSelection {
                provider: "mock".to_string(),
                model: model.to_string(),
            },
            InferenceProviderMetadata::local("mock"),
            ModelDescriptor {
                id: model.to_string(),
                name: model.to_string(),
                context_window: Some(128_000),
                default_reasoning: Some(reasoning.to_string()),
                supported_reasoning: vec![ReasoningEffortDescriptor {
                    effort: reasoning.to_string(),
                    description: reasoning.to_string(),
                }],
            },
            InferenceCapabilities {
                image_input,
                ..InferenceCapabilities::coding_agent_default()
            },
        )
    }
}
