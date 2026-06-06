use serde::{Deserialize, Serialize};

use crate::extension::InferenceRouterId;
use crate::inference::{
    InferenceCapabilities, InferenceProviderMetadata, ModelDescriptor, ModelSelection,
    ReasoningConfig, RuntimeProfile, SpeedPolicyPhase,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelSelectionMode {
    Manual {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    Auto {
        option_id: String,
        router_id: InferenceRouterId,
        label: String,
        baseline: ModelSelection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
}

impl ModelSelectionMode {
    pub fn manual(
        provider: impl Into<String>,
        model: impl Into<String>,
        reasoning: Option<String>,
    ) -> Self {
        Self::Manual {
            provider: provider.into(),
            model: model.into(),
            reasoning,
        }
    }

    pub fn auto(
        option_id: impl Into<String>,
        router_id: impl Into<String>,
        label: impl Into<String>,
        baseline: ModelSelection,
        profile: Option<String>,
        reasoning: Option<String>,
    ) -> Self {
        Self::Auto {
            option_id: option_id.into(),
            router_id: router_id.into(),
            label: label.into(),
            baseline,
            profile,
            reasoning,
        }
    }

    pub fn concrete_selection(&self) -> ModelSelection {
        match self {
            Self::Manual {
                provider, model, ..
            } => ModelSelection {
                provider: provider.clone(),
                model: model.clone(),
            },
            Self::Auto { baseline, .. } => baseline.clone(),
        }
    }

    pub fn reasoning(&self) -> Option<&str> {
        match self {
            Self::Manual { reasoning, .. } | Self::Auto { reasoning, .. } => reasoning.as_deref(),
        }
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingOptionDescriptor {
    pub id: String,
    pub label: String,
    pub router_id: InferenceRouterId,
    pub baseline: ModelSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_true() -> bool {
    true
}

impl InferenceRoutingOptionDescriptor {
    pub fn selectable(
        id: impl Into<String>,
        label: impl Into<String>,
        router_id: impl Into<String>,
        baseline: ModelSelection,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            router_id: router_id.into(),
            baseline,
            profile: None,
            objective: None,
            reasoning: None,
            available: true,
            unavailable_reason: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn unavailable(mut self, reason: impl Into<String>) -> Self {
        self.available = false;
        self.unavailable_reason = Some(reason.into());
        self
    }

    pub fn selection_mode(&self) -> ModelSelectionMode {
        ModelSelectionMode::auto(
            self.id.clone(),
            self.router_id.clone(),
            self.label.clone(),
            self.baseline.clone(),
            self.profile.clone(),
            self.reasoning.clone(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingContext {
    pub thread_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub round_index: u32,
    #[serde(default)]
    pub runtime_profile: RuntimeProfile,
    pub default_selection: ModelSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_selection: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SpeedPolicyPhase>,
    #[serde(default)]
    pub transcript: InferenceRoutingTranscriptSummary,
    #[serde(default)]
    pub tools: InferenceRoutingToolSummary,
    #[serde(default)]
    pub candidates: Vec<InferenceRoutingCandidate>,
    #[serde(default)]
    pub signals: Vec<InferenceRoutingSignal>,
    #[serde(default)]
    pub prior_failures: u32,
    #[serde(default)]
    pub prior_escalations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingTranscriptSummary {
    #[serde(default)]
    pub item_count: u32,
    #[serde(default)]
    pub user_message_count: u32,
    #[serde(default)]
    pub assistant_message_count: u32,
    #[serde(default)]
    pub tool_result_count: u32,
    #[serde(default)]
    pub has_image_input: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_user_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_tool_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximate_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingToolSummary {
    #[serde(default)]
    pub available_count: u32,
    #[serde(default)]
    pub has_file_tools: bool,
    #[serde(default)]
    pub has_shell_tools: bool,
    #[serde(default)]
    pub has_network_tools: bool,
    #[serde(default)]
    pub requires_tool_calls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingCandidate {
    pub selection: ModelSelection,
    pub provider: InferenceProviderMetadata,
    pub model: ModelDescriptor,
    pub capabilities: InferenceCapabilities,
    #[serde(default)]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl InferenceRoutingCandidate {
    pub fn available(
        selection: ModelSelection,
        provider: InferenceProviderMetadata,
        model: ModelDescriptor,
        capabilities: InferenceCapabilities,
    ) -> Self {
        Self {
            selection,
            provider,
            model,
            capabilities,
            available: true,
            unavailable_reason: None,
        }
    }

    pub fn unavailable(
        selection: ModelSelection,
        provider: InferenceProviderMetadata,
        model: ModelDescriptor,
        capabilities: InferenceCapabilities,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            selection,
            provider,
            model,
            capabilities,
            available: false,
            unavailable_reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingSignal {
    pub key: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

impl InferenceRoutingSignal {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            source: None,
            weight: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceRoutingOutcome {
    Selected,
    Escalated,
    Fallback,
    #[default]
    Abstained,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingDecision {
    pub router_id: InferenceRouterId,
    pub outcome: InferenceRoutingOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub matched_signals: Vec<InferenceRoutingSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_delta: Option<InferenceRoutingCostDelta>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl InferenceRoutingDecision {
    pub fn selected(
        router_id: impl Into<String>,
        selection: ModelSelection,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            router_id: router_id.into(),
            outcome: InferenceRoutingOutcome::Selected,
            selected: Some(selection),
            reasoning: None,
            reason: reason.into(),
            confidence: None,
            matched_signals: Vec::new(),
            baseline: None,
            cost_delta: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn abstain(router_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            router_id: router_id.into(),
            outcome: InferenceRoutingOutcome::Abstained,
            selected: None,
            reasoning: None,
            reason: reason.into(),
            confidence: None,
            matched_signals: Vec::new(),
            baseline: None,
            cost_delta: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn fallback(router_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            router_id: router_id.into(),
            outcome: InferenceRoutingOutcome::Fallback,
            selected: None,
            reasoning: None,
            reason: reason.into(),
            confidence: None,
            matched_signals: Vec::new(),
            baseline: None,
            cost_delta: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn escalated(
        router_id: impl Into<String>,
        selection: ModelSelection,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            router_id: router_id.into(),
            outcome: InferenceRoutingOutcome::Escalated,
            selected: Some(selection),
            reasoning: None,
            reason: reason.into(),
            confidence: None,
            matched_signals: Vec::new(),
            baseline: None,
            cost_delta: None,
            metadata: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingCostDelta {
    pub selected_estimate: InferenceRoutingCostEstimate,
    pub baseline_estimate: InferenceRoutingCostEstimate,
    pub estimated_savings_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_overhead_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRoutingCostEstimate {
    pub selection: ModelSelection,
    pub prompt_cost_usd: f64,
    pub completion_cost_usd: f64,
    pub total_cost_usd: f64,
    pub price_source: String,
    pub usage_source: String,
    #[serde(default)]
    pub incomplete: bool,
}

#[async_trait::async_trait]
pub trait InferenceRouter: Send + Sync + 'static {
    fn id(&self) -> InferenceRouterId;

    fn routing_options(&self) -> Vec<InferenceRoutingOptionDescriptor> {
        Vec::new()
    }

    async fn route(
        &self,
        context: InferenceRoutingContext,
    ) -> anyhow::Result<InferenceRoutingDecision>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{ProviderAuthType, ReasoningEffortDescriptor};

    #[test]
    fn routing_context_serializes_camel_case_fields() {
        let context = InferenceRoutingContext {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            round_index: 2,
            runtime_profile: RuntimeProfile::Interactive,
            default_selection: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-5.4".to_string(),
            },
            requested_selection: None,
            phase: Some(SpeedPolicyPhase::Verification),
            transcript: InferenceRoutingTranscriptSummary {
                item_count: 3,
                has_image_input: true,
                latest_user_message_preview: Some("review auth changes".to_string()),
                ..InferenceRoutingTranscriptSummary::default()
            },
            tools: InferenceRoutingToolSummary {
                available_count: 8,
                requires_tool_calls: true,
                ..InferenceRoutingToolSummary::default()
            },
            candidates: vec![candidate("openai", "gpt-5.4")],
            signals: vec![InferenceRoutingSignal::new("phase", "verification")],
            prior_failures: 1,
            prior_escalations: 0,
            estimated_input_tokens: Some(4096),
        };

        let value = serde_json::to_value(&context).expect("serialize context");

        assert_eq!(value["threadId"], "thread-1");
        assert_eq!(value["turnId"], "turn-1");
        assert_eq!(value["roundIndex"], 2);
        assert_eq!(value["defaultSelection"]["provider"], "openai");
        assert_eq!(value["phase"], "verification");
        assert_eq!(value["transcript"]["hasImageInput"], true);
        assert_eq!(
            value["transcript"]["latestUserMessagePreview"],
            "review auth changes"
        );
        assert_eq!(value["tools"]["requiresToolCalls"], true);
        assert_eq!(value["estimatedInputTokens"], 4096);
        assert_eq!(value["candidates"][0]["selection"]["model"], "gpt-5.4");
    }

    #[test]
    fn routing_decision_serializes_selected_abstain_and_fallback() {
        let selected = InferenceRoutingDecision {
            reasoning: Some(ReasoningConfig {
                enabled: true,
                level: Some("low".to_string()),
            }),
            confidence: Some(0.82),
            matched_signals: vec![InferenceRoutingSignal::new("intent", "file_lookup")],
            baseline: Some(ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-5.4".to_string(),
            }),
            ..InferenceRoutingDecision::selected(
                "local-router",
                ModelSelection {
                    provider: "openai".to_string(),
                    model: "gpt-5.4-mini".to_string(),
                },
                "routine lookup",
            )
        };
        let selected_value = serde_json::to_value(selected).expect("serialize selected decision");

        assert_eq!(selected_value["routerId"], "local-router");
        assert_eq!(selected_value["outcome"], "selected");
        assert_eq!(selected_value["selected"]["model"], "gpt-5.4-mini");
        assert_eq!(selected_value["reasoning"]["level"], "low");
        assert_eq!(selected_value["matchedSignals"][0]["key"], "intent");

        let abstain = serde_json::to_value(InferenceRoutingDecision::abstain(
            "local-router",
            "no safe candidate",
        ))
        .expect("serialize abstain decision");
        assert_eq!(abstain["outcome"], "abstained");
        assert_eq!(abstain["reason"], "no safe candidate");
        assert!(abstain.get("selected").is_none());

        let fallback = serde_json::to_value(InferenceRoutingDecision::fallback(
            "local-router",
            "invalid router decision",
        ))
        .expect("serialize fallback decision");
        assert_eq!(fallback["outcome"], "fallback");
        assert_eq!(fallback["reason"], "invalid router decision");
    }

    #[test]
    fn routing_option_descriptor_round_trips_with_selection_mode() {
        let option = InferenceRoutingOptionDescriptor {
            profile: Some("coding".to_string()),
            objective: Some("minimize latency without losing code quality".to_string()),
            reasoning: Some("low".to_string()),
            metadata: serde_json::json!({ "source": "test" }),
            ..InferenceRoutingOptionDescriptor::selectable(
                "local-router:coding",
                "Auto: Coding",
                "local-router",
                ModelSelection {
                    provider: "codex".to_string(),
                    model: "gpt-5.5".to_string(),
                },
            )
        };

        let value = serde_json::to_value(&option).expect("serialize routing option");

        assert_eq!(value["id"], "local-router:coding");
        assert_eq!(value["label"], "Auto: Coding");
        assert_eq!(value["routerId"], "local-router");
        assert_eq!(value["baseline"]["provider"], "codex");
        assert_eq!(value["baseline"]["model"], "gpt-5.5");
        assert_eq!(value["profile"], "coding");
        assert_eq!(
            value["objective"],
            "minimize latency without losing code quality"
        );
        assert_eq!(value["reasoning"], "low");
        assert_eq!(value["available"], true);

        let round_trip: InferenceRoutingOptionDescriptor =
            serde_json::from_value(value).expect("deserialize routing option");
        assert_eq!(round_trip, option);

        assert_eq!(
            round_trip.selection_mode(),
            ModelSelectionMode::Auto {
                option_id: "local-router:coding".to_string(),
                router_id: "local-router".to_string(),
                label: "Auto: Coding".to_string(),
                baseline: ModelSelection {
                    provider: "codex".to_string(),
                    model: "gpt-5.5".to_string(),
                },
                profile: Some("coding".to_string()),
                reasoning: Some("low".to_string()),
            }
        );
    }

    fn candidate(provider: &str, model: &str) -> InferenceRoutingCandidate {
        InferenceRoutingCandidate::available(
            ModelSelection {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            InferenceProviderMetadata {
                name: provider.to_string(),
                description: None,
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("API key".to_string()),
                auth_configured: Some(true),
                recommended: true,
                sort_order: 10,
            },
            ModelDescriptor {
                id: model.to_string(),
                name: model.to_string(),
                context_window: Some(128_000),
                default_reasoning: Some("medium".to_string()),
                supported_reasoning: vec![ReasoningEffortDescriptor {
                    effort: "low".to_string(),
                    description: "Low".to_string(),
                }],
            },
            InferenceCapabilities::coding_agent_default(),
        )
    }
}
