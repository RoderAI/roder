use roder_api::skills::{
    SkillActivationState, SkillDescriptor, SkillExposure, SkillSelector, SkillSource,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfig {
    #[serde(default)]
    pub config: Vec<SkillConfigRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfigRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<SkillExposure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSkillConfig {
    pub activation: SkillActivationState,
    pub exposure: SkillExposure,
    pub diagnostics: Vec<String>,
}

impl SkillConfigRule {
    pub fn from_selector(selector: SkillSelector) -> Self {
        match selector {
            SkillSelector::Name { name } => Self {
                name: Some(name),
                path: None,
                enabled: None,
                exposure: None,
            },
            SkillSelector::Path { path } => Self {
                name: None,
                path: Some(path),
                enabled: None,
                exposure: None,
            },
        }
    }

    pub fn selector(&self) -> Option<SkillSelector> {
        if let Some(path) = &self.path {
            return Some(SkillSelector::Path { path: path.clone() });
        }
        self.name
            .as_ref()
            .map(|name| SkillSelector::Name { name: name.clone() })
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = Some(enabled);
    }

    pub fn set_exposure(&mut self, exposure: SkillExposure) {
        self.exposure = Some(exposure);
    }

    fn matches(&self, descriptor: &SkillDescriptor) -> bool {
        self.path
            .as_ref()
            .is_some_and(|path| path == &descriptor.canonical_path)
            || self
                .name
                .as_ref()
                .is_some_and(|name| name == &descriptor.name)
    }
}

impl SkillsConfig {
    pub fn upsert_rule(
        &mut self,
        selector: SkillSelector,
        update: impl FnOnce(&mut SkillConfigRule),
    ) {
        if let Some(rule) = self
            .config
            .iter_mut()
            .find(|rule| rule.selector() == Some(selector.clone()))
        {
            update(rule);
            return;
        }
        let mut rule = SkillConfigRule::from_selector(selector);
        update(&mut rule);
        self.config.push(rule);
    }
}

pub fn apply_skill_config(
    descriptor: &SkillDescriptor,
    rules: &[SkillConfigRule],
) -> AppliedSkillConfig {
    let mut activation = descriptor.activation;
    let mut exposure = descriptor.exposure;
    let mut diagnostics = Vec::new();
    for rule in rules.iter().filter(|rule| rule.matches(descriptor)) {
        if let Some(enabled) = rule.enabled {
            activation = if enabled {
                if descriptor.experimental {
                    SkillActivationState::Experimental
                } else {
                    SkillActivationState::Enabled
                }
            } else {
                SkillActivationState::Disabled
            };
        }
        if let Some(next) = rule.exposure {
            exposure = next;
        }
    }
    if descriptor.source == SkillSource::BuiltIn && activation == SkillActivationState::Disabled {
        diagnostics.push("built-in skill disabled by config".to_string());
    }
    AppliedSkillConfig {
        activation,
        exposure,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::skills::SkillDescriptor;

    use super::*;

    fn descriptor() -> SkillDescriptor {
        SkillDescriptor {
            id: "builtin:roder-builtin://commit/SKILL.md".to_string(),
            name: "commit".to_string(),
            canonical_path: "roder-builtin://commit/SKILL.md".to_string(),
            source: SkillSource::BuiltIn,
            exposure: SkillExposure::DirectOnly,
            activation: SkillActivationState::Enabled,
            description: "Commit safely".to_string(),
            short_description: None,
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        }
    }

    #[test]
    fn config_applies_name_and_path_rules() {
        let rules = vec![
            SkillConfigRule {
                name: Some("commit".to_string()),
                path: None,
                enabled: Some(false),
                exposure: None,
            },
            SkillConfigRule {
                name: None,
                path: Some("roder-builtin://commit/SKILL.md".to_string()),
                enabled: Some(true),
                exposure: Some(SkillExposure::Global),
            },
        ];

        let applied = apply_skill_config(&descriptor(), &rules);
        assert_eq!(applied.activation, SkillActivationState::Enabled);
        assert_eq!(applied.exposure, SkillExposure::Global);
    }

    #[test]
    fn config_round_trips_from_toml_shape() {
        let config: SkillsConfig = toml::from_str(
            r#"
[[config]]
name = "commit"
enabled = true
exposure = "direct_only"

[[config]]
path = "roder-builtin://commit/SKILL.md"
enabled = false
"#,
        )
        .unwrap();

        assert_eq!(config.config.len(), 2);
        assert_eq!(
            config.config[0].selector(),
            Some(SkillSelector::Name {
                name: "commit".to_string()
            })
        );
    }

    #[test]
    fn config_upserts_rules_by_selector() {
        let mut config = SkillsConfig::default();
        config.upsert_rule(
            SkillSelector::Name {
                name: "commit".to_string(),
            },
            |rule| rule.set_enabled(false),
        );
        config.upsert_rule(
            SkillSelector::Name {
                name: "commit".to_string(),
            },
            |rule| rule.set_exposure(SkillExposure::Global),
        );

        assert_eq!(config.config.len(), 1);
        assert_eq!(config.config[0].enabled, Some(false));
        assert_eq!(config.config[0].exposure, Some(SkillExposure::Global));
    }
}
