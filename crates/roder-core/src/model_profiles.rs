use std::collections::HashMap;

use roder_api::catalog::{
    EDIT_TOOL_EDIT, EDIT_TOOL_PATCH, REASONING_HIGH, REASONING_LOW, REASONING_MAX,
    REASONING_MEDIUM, REASONING_MINIMAL, REASONING_NONE, REASONING_ULTRA, REASONING_XHIGH,
    built_in_model_profile, built_in_model_profiles, lookup_model, model_supports_reasoning_effort,
};
use roder_api::inference::{
    ModelHarnessProfile, ModelInstructionOverlay, ModelProfileReasoning, ModelSchemaPolicy,
    ProviderFamily,
};

#[derive(Debug, Clone, Default)]
pub struct ModelProfileOverrides {
    pub profiles: HashMap<String, ModelHarnessProfileOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelHarnessProfileOverride {
    pub provider_family: Option<String>,
    pub edit_tool: Option<String>,
    pub schema_policy: Option<String>,
    pub instruction_overlay: Option<String>,
    pub reasoning: ModelProfileReasoningOverride,
    pub parallel_tool_calls: Option<bool>,
    pub auto_compact_token_limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelProfileReasoningOverride {
    pub orientation: Option<String>,
    pub execution: Option<String>,
    pub verification: Option<String>,
    pub recovery: Option<String>,
}

pub fn resolve_model_profiles(
    overrides: &ModelProfileOverrides,
) -> anyhow::Result<HashMap<String, ModelHarnessProfile>> {
    let mut profiles = HashMap::new();
    for profile in built_in_model_profiles() {
        profiles.entry(profile.model.clone()).or_insert(profile);
    }

    for (model, override_profile) in &overrides.profiles {
        let mut profile = profiles
            .remove(model)
            .or_else(|| built_in_model_profile(model))
            .ok_or_else(|| anyhow::anyhow!("unknown model profile {model:?}"))?;
        apply_override(&mut profile, override_profile)?;
        validate_model_profile(&profile)?;
        profiles.insert(model.clone(), profile);
    }

    for profile in profiles.values() {
        validate_model_profile(profile)?;
    }

    Ok(profiles)
}

fn apply_override(
    profile: &mut ModelHarnessProfile,
    override_profile: &ModelHarnessProfileOverride,
) -> anyhow::Result<()> {
    if let Some(provider_family) = &override_profile.provider_family {
        profile.provider_family = parse_provider_family(provider_family)?;
    }
    if let Some(edit_tool) = &override_profile.edit_tool {
        profile.edit_tool = Some(edit_tool.clone());
    }
    if let Some(schema_policy) = &override_profile.schema_policy {
        profile.schema_policy = parse_schema_policy(schema_policy)?;
    }
    if let Some(instruction_overlay) = &override_profile.instruction_overlay {
        profile.instruction_overlay = parse_instruction_overlay(instruction_overlay)?;
    }
    merge_reasoning(&mut profile.reasoning, &override_profile.reasoning);
    if let Some(parallel_tool_calls) = override_profile.parallel_tool_calls {
        profile.parallel_tool_calls = Some(parallel_tool_calls);
    }
    if let Some(limit) = override_profile.auto_compact_token_limit {
        profile.auto_compact_token_limit = Some(limit);
    }
    Ok(())
}

fn merge_reasoning(
    profile: &mut ModelProfileReasoning,
    override_reasoning: &ModelProfileReasoningOverride,
) {
    if let Some(reasoning) = &override_reasoning.orientation {
        profile.orientation = Some(reasoning.clone());
    }
    if let Some(reasoning) = &override_reasoning.execution {
        profile.execution = Some(reasoning.clone());
    }
    if let Some(reasoning) = &override_reasoning.verification {
        profile.verification = Some(reasoning.clone());
    }
    if let Some(reasoning) = &override_reasoning.recovery {
        profile.recovery = Some(reasoning.clone());
    }
}

pub fn validate_model_profile(profile: &ModelHarnessProfile) -> anyhow::Result<()> {
    if let Some(edit_tool) = profile.edit_tool.as_deref() {
        validate_edit_tool_name(edit_tool)?;
        if !lookup_model(&profile.model)
            .map(|model| model.supports_tools)
            .unwrap_or(false)
        {
            anyhow::bail!(
                "model profile {:?} sets edit_tool but model does not support tools",
                profile.model
            );
        }
    }

    for (phase, effort) in [
        ("orientation", profile.reasoning.orientation.as_deref()),
        ("execution", profile.reasoning.execution.as_deref()),
        ("verification", profile.reasoning.verification.as_deref()),
        ("recovery", profile.reasoning.recovery.as_deref()),
    ] {
        let Some(effort) = effort else {
            continue;
        };
        validate_reasoning_name(effort)?;
        if effort != REASONING_NONE && !model_supports_reasoning_effort(&profile.model, effort) {
            anyhow::bail!(
                "model profile {:?} uses unsupported {phase} reasoning effort {:?}",
                profile.model,
                effort
            );
        }
    }

    if profile.parallel_tool_calls == Some(true)
        && !lookup_model(&profile.model)
            .map(|model| model.supports_tools)
            .unwrap_or(false)
    {
        anyhow::bail!(
            "model profile {:?} enables parallel_tool_calls but model does not support tools",
            profile.model
        );
    }

    Ok(())
}

fn validate_edit_tool_name(value: &str) -> anyhow::Result<()> {
    match value {
        EDIT_TOOL_PATCH | EDIT_TOOL_EDIT => Ok(()),
        other => anyhow::bail!(
            "unsupported model profile edit_tool {other:?}; expected {EDIT_TOOL_PATCH:?} or {EDIT_TOOL_EDIT:?}"
        ),
    }
}

fn validate_reasoning_name(value: &str) -> anyhow::Result<()> {
    match value {
        REASONING_NONE | REASONING_MINIMAL | REASONING_LOW | REASONING_MEDIUM | REASONING_HIGH
        | REASONING_XHIGH | REASONING_MAX | REASONING_ULTRA => Ok(()),
        other => anyhow::bail!("unsupported model profile reasoning effort {other:?}"),
    }
}

fn parse_provider_family(value: &str) -> anyhow::Result<ProviderFamily> {
    match value {
        "mock" => Ok(ProviderFamily::Mock),
        "openai" => Ok(ProviderFamily::OpenAi),
        "anthropic" => Ok(ProviderFamily::Anthropic),
        "gemini" => Ok(ProviderFamily::Gemini),
        "xai" => Ok(ProviderFamily::Xai),
        "opencode" => Ok(ProviderFamily::Opencode),
        "poolside" => Ok(ProviderFamily::Poolside),
        other => anyhow::bail!("unsupported model profile provider_family {other:?}"),
    }
}

fn parse_schema_policy(value: &str) -> anyhow::Result<ModelSchemaPolicy> {
    match value {
        "standard_required_first" => Ok(ModelSchemaPolicy::StandardRequiredFirst),
        "required_first_flat" => Ok(ModelSchemaPolicy::RequiredFirstFlat),
        other => anyhow::bail!("unsupported model profile schema_policy {other:?}"),
    }
}

fn parse_instruction_overlay(value: &str) -> anyhow::Result<ModelInstructionOverlay> {
    match value {
        "standard" => Ok(ModelInstructionOverlay::Standard),
        "literal_tool_outputs" => Ok(ModelInstructionOverlay::LiteralToolOutputs),
        "intuitive_context" => Ok(ModelInstructionOverlay::IntuitiveContext),
        other => anyhow::bail!("unsupported model profile instruction_overlay {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_profile_defaults_include_catalog_values() {
        let profiles = resolve_model_profiles(&ModelProfileOverrides::default()).unwrap();
        let profile = profiles.get("gpt-5.5").unwrap();

        assert_eq!(profile.provider_family, ProviderFamily::OpenAi);
        assert_eq!(profile.edit_tool.as_deref(), Some(EDIT_TOOL_PATCH));
        assert_eq!(profile.reasoning.execution.as_deref(), Some(REASONING_LOW));
    }

    #[test]
    fn model_profile_overrides_merge_into_builtin_profile() {
        let mut overrides = ModelProfileOverrides::default();
        overrides.profiles.insert(
            "gpt-5.5".to_string(),
            ModelHarnessProfileOverride {
                schema_policy: Some("standard_required_first".to_string()),
                reasoning: ModelProfileReasoningOverride {
                    execution: Some("medium".to_string()),
                    ..Default::default()
                },
                parallel_tool_calls: Some(false),
                auto_compact_token_limit: Some(180_000),
                ..Default::default()
            },
        );

        let profiles = resolve_model_profiles(&overrides).unwrap();
        let profile = profiles.get("gpt-5.5").unwrap();

        assert_eq!(
            profile.schema_policy,
            ModelSchemaPolicy::StandardRequiredFirst
        );
        assert_eq!(
            profile.reasoning.execution.as_deref(),
            Some(REASONING_MEDIUM)
        );
        assert_eq!(profile.parallel_tool_calls, Some(false));
        assert_eq!(profile.auto_compact_token_limit, Some(180_000));
    }

    #[test]
    fn model_profile_validation_rejects_unknown_edit_tool() {
        let mut overrides = ModelProfileOverrides::default();
        overrides.profiles.insert(
            "gpt-5.5".to_string(),
            ModelHarnessProfileOverride {
                edit_tool: Some("legacy_replace".to_string()),
                ..Default::default()
            },
        );

        let err = resolve_model_profiles(&overrides).unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported model profile edit_tool")
        );
    }

    #[test]
    fn model_profile_validation_rejects_unsupported_reasoning() {
        let mut overrides = ModelProfileOverrides::default();
        overrides.profiles.insert(
            "mock".to_string(),
            ModelHarnessProfileOverride {
                reasoning: ModelProfileReasoningOverride {
                    execution: Some("high".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let err = resolve_model_profiles(&overrides).unwrap_err();
        assert!(err.to_string().contains("unsupported execution reasoning"));
    }

    #[test]
    fn model_profile_validation_accepts_xhigh_for_supported_models() {
        let mut overrides = ModelProfileOverrides::default();
        overrides.profiles.insert(
            "gpt-5.5".to_string(),
            ModelHarnessProfileOverride {
                reasoning: ModelProfileReasoningOverride {
                    orientation: Some(REASONING_XHIGH.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let profiles = resolve_model_profiles(&overrides).unwrap();
        assert_eq!(
            profiles
                .get("gpt-5.5")
                .unwrap()
                .reasoning
                .orientation
                .as_deref(),
            Some(REASONING_XHIGH)
        );
    }

    #[test]
    fn model_profile_validation_accepts_ultra_for_supported_models() {
        let mut overrides = ModelProfileOverrides::default();
        overrides.profiles.insert(
            "gpt-5.6-sol".to_string(),
            ModelHarnessProfileOverride {
                reasoning: ModelProfileReasoningOverride {
                    orientation: Some(REASONING_ULTRA.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let profiles = resolve_model_profiles(&overrides).unwrap();
        assert_eq!(
            profiles
                .get("gpt-5.6-sol")
                .unwrap()
                .reasoning
                .orientation
                .as_deref(),
            Some(REASONING_ULTRA)
        );
    }
}
