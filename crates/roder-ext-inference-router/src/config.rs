use std::collections::HashMap;

use anyhow::Context;
use roder_api::inference::ModelSelection;
use roder_api::inference_routing::InferenceRoutingOptionDescriptor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LocalInferenceRouterConfig {
    #[serde(default)]
    pub enabled: bool,
    pub profile: Option<String>,
    pub baseline_provider: Option<String>,
    pub baseline_model: Option<String>,
    pub objective: Option<String>,
    #[serde(default)]
    pub tiers: HashMap<String, InferenceRouterTierConfig>,
    #[serde(default)]
    pub profiles: HashMap<String, InferenceRouterProfileConfig>,
    #[serde(default)]
    pub prices: HashMap<String, InferenceRouterPriceConfig>,
    #[serde(default)]
    pub classifier_comparison: InferenceRouterClassifierComparisonConfig,
}

impl LocalInferenceRouterConfig {
    pub fn from_router_parts(
        enabled: bool,
        profile: Option<String>,
        baseline_provider: Option<String>,
        baseline_model: Option<String>,
        extension: serde_json::Value,
    ) -> anyhow::Result<Self> {
        let policy = match extension {
            serde_json::Value::Null => LocalInferenceRouterPolicyConfig::default(),
            value => serde_json::from_value::<LocalInferenceRouterPolicyConfig>(value)
                .context("invalid local inference_router.extension config")?,
        };

        Ok(Self {
            enabled,
            profile,
            baseline_provider,
            baseline_model,
            objective: policy.objective,
            tiers: policy.tiers,
            profiles: policy.profiles,
            prices: policy.prices,
            classifier_comparison: policy.classifier_comparison,
        })
    }

    pub fn routing_options(&self, router_id: &str) -> Vec<InferenceRoutingOptionDescriptor> {
        if !self.enabled {
            return Vec::new();
        }
        let Some(baseline) = self.baseline_selection() else {
            return Vec::new();
        };
        let profile = self.active_profile_name();
        let profile_config = profile.and_then(|name| self.profiles.get(name));
        let label = profile
            .map(|profile| format!("Auto: {}", title_case_label(profile)))
            .unwrap_or_else(|| "Auto: Local Router".to_string());
        let id = profile
            .map(|profile| format!("{router_id}:{profile}"))
            .unwrap_or_else(|| format!("{router_id}:default"));
        let mut option =
            InferenceRoutingOptionDescriptor::selectable(id, label, router_id, baseline);
        option.profile = profile.map(str::to_string);
        option.objective = profile_config
            .and_then(|profile| profile.objective.clone())
            .or_else(|| self.objective.clone());
        option.metadata = serde_json::json!({ "router": router_id });
        vec![option]
    }

    fn baseline_selection(&self) -> Option<ModelSelection> {
        Some(ModelSelection {
            provider: self.baseline_provider.clone()?,
            model: self.baseline_model.clone()?,
        })
    }

    fn active_profile_name(&self) -> Option<&str> {
        if let Some(profile) = self
            .profile
            .as_deref()
            .filter(|profile| !profile.trim().is_empty())
        {
            return Some(profile);
        }
        if self.profiles.len() == 1 {
            return self.profiles.keys().next().map(String::as_str);
        }
        None
    }
}

fn title_case_label(value: &str) -> String {
    value
        .split(['-', '_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct LocalInferenceRouterPolicyConfig {
    pub objective: Option<String>,
    #[serde(default)]
    pub tiers: HashMap<String, InferenceRouterTierConfig>,
    #[serde(default)]
    pub profiles: HashMap<String, InferenceRouterProfileConfig>,
    #[serde(default)]
    pub prices: HashMap<String, InferenceRouterPriceConfig>,
    #[serde(default)]
    pub classifier_comparison: InferenceRouterClassifierComparisonConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceRouterTierConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceRouterProfileConfig {
    pub objective: Option<String>,
    pub default_tier: Option<String>,
    pub simple_tier: Option<String>,
    pub standard_tier: Option<String>,
    pub strong_tier: Option<String>,
    pub risk_floor_tier: Option<String>,
    #[serde(default)]
    pub risk_floors: HashMap<String, String>,
    pub classifier_prompt: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InferenceRouterPriceConfig {
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub cached_input_per_million: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceRouterClassifierComparisonConfig {
    #[serde(default)]
    pub enabled: bool,
    pub prompt: Option<String>,
    pub label: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_policy_from_generic_extension_value() {
        let config = LocalInferenceRouterConfig::from_router_parts(
            true,
            Some("coding".to_string()),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
            serde_json::json!({
                "objective": "cost",
                "tiers": {
                    "simple": {
                        "provider": "codex",
                        "model": "gpt-5.4-mini",
                        "reasoning": "low"
                    }
                },
                "profiles": {
                    "coding": {
                        "default_tier": "simple",
                        "risk_floors": {
                            "security": "strong"
                        }
                    }
                },
                "prices": {
                    "codex/gpt-5.4-mini": {
                        "input_per_million": 0.25
                    }
                },
                "classifier_comparison": {
                    "enabled": true,
                    "label": "classifier-v2"
                }
            }),
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.profile.as_deref(), Some("coding"));
        assert_eq!(config.objective.as_deref(), Some("cost"));
        assert_eq!(
            config
                .profiles
                .get("coding")
                .and_then(|profile| profile.risk_floors.get("security"))
                .map(String::as_str),
            Some("strong")
        );
        assert_eq!(
            config
                .prices
                .get("codex/gpt-5.4-mini")
                .and_then(|price| price.input_per_million),
            Some(0.25)
        );
        assert!(config.classifier_comparison.enabled);
    }

    #[test]
    fn exposes_auto_option_for_enabled_router_with_baseline() {
        let config = LocalInferenceRouterConfig::from_router_parts(
            true,
            Some("coding".to_string()),
            Some("codex".to_string()),
            Some("gpt-5.5".to_string()),
            serde_json::json!({
                "objective": "spend carefully",
                "profiles": {
                    "coding": {
                        "objective": "coding latency"
                    }
                }
            }),
        )
        .unwrap();

        let options = config.routing_options("local-inference-router");

        assert_eq!(options.len(), 1);
        assert_eq!(options[0].id, "local-inference-router:coding");
        assert_eq!(options[0].label, "Auto: Coding");
        assert_eq!(options[0].baseline.provider, "codex");
        assert_eq!(options[0].baseline.model, "gpt-5.5");
        assert_eq!(options[0].profile.as_deref(), Some("coding"));
        assert_eq!(options[0].objective.as_deref(), Some("coding latency"));
        assert_eq!(options[0].metadata["router"], "local-inference-router");
    }

    #[test]
    fn omits_auto_option_without_safe_baseline() {
        let config = LocalInferenceRouterConfig::from_router_parts(
            true,
            Some("coding".to_string()),
            Some("codex".to_string()),
            None,
            serde_json::json!({
                "profiles": {
                    "coding": {}
                }
            }),
        )
        .unwrap();

        assert!(config.routing_options("local-inference-router").is_empty());
    }
}
