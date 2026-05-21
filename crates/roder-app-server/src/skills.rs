use roder_api::skills::{Skill, SkillDescriptor, SkillSelector};
use roder_protocol::{
    JsonRpcError, SkillsListResult, SkillsReadParams, SkillsReadResult, SkillsSetEnabledParams,
    SkillsSetExposureParams, SkillsUpdateResult,
};
use roder_skills::SkillConfigRule;

use crate::server::AppServer;

impl AppServer {
    pub(crate) async fn handle_skills_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let registry = self.runtime.skills_snapshot().await;
        json_result(SkillsListResult {
            skills: registry
                .skills()
                .iter()
                .map(|skill| skill.descriptor.clone())
                .collect(),
            diagnostics: registry.diagnostics().to_vec(),
        })
    }

    pub(crate) async fn handle_skills_read(
        &self,
        params: SkillsReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let registry = self.runtime.skills_snapshot().await;
        json_result(SkillsReadResult {
            skill: find_skill(registry.skills(), &params.selector).cloned(),
        })
    }

    pub(crate) async fn handle_skills_set_enabled(
        &self,
        params: SkillsSetEnabledParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let selector = self.resolve_mutation_selector(&params.selector).await?;
        self.update_skills_config(selector, |rule| rule.set_enabled(params.enabled))
            .await
    }

    pub(crate) async fn handle_skills_set_exposure(
        &self,
        params: SkillsSetExposureParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let selector = self.resolve_mutation_selector(&params.selector).await?;
        self.update_skills_config(selector, |rule| rule.set_exposure(params.exposure))
            .await
    }

    async fn resolve_mutation_selector(
        &self,
        selector: &SkillSelector,
    ) -> Result<SkillSelector, JsonRpcError> {
        let registry = self.runtime.skills_snapshot().await;
        match matching_descriptors(registry.skills(), selector).as_slice() {
            [] => Err(invalid_params("skill not found")),
            [descriptor] => Ok(SkillSelector::Path {
                path: descriptor.canonical_path.clone(),
            }),
            matches => Err(invalid_params(format!(
                "skill name is ambiguous; select by path: {}",
                matches
                    .iter()
                    .map(|descriptor| descriptor.canonical_path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    async fn update_skills_config(
        &self,
        selector: SkillSelector,
        update: impl FnOnce(&mut SkillConfigRule),
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut cfg = roder_config::load_config().map_err(internal_error)?;
        let mut skills = cfg.skills.take().unwrap_or_default();
        skills.upsert_rule(selector, update);
        if self.persist_user_config_enabled() {
            roder_config::save_skills_config(&skills).map_err(internal_error)?;
        }
        let registry = roder_config::build_skills_registry(self.runtime.workspace(), Some(&skills));
        let skills_result = registry
            .skills()
            .iter()
            .map(|skill| skill.descriptor.clone())
            .collect();
        let diagnostics = registry.diagnostics().to_vec();
        self.runtime.set_skills(registry).await;
        json_result(SkillsUpdateResult {
            skills: skills_result,
            diagnostics,
        })
    }
}

fn find_skill<'a>(skills: &'a [Skill], selector: &SkillSelector) -> Option<&'a Skill> {
    let matches = matching_skills(skills, selector);
    (matches.len() == 1).then(|| matches[0])
}

fn matching_skills<'a>(skills: &'a [Skill], selector: &SkillSelector) -> Vec<&'a Skill> {
    skills
        .iter()
        .filter(|skill| match selector {
            SkillSelector::Name { name } => skill.descriptor.name == *name,
            SkillSelector::Path { path } => skill.descriptor.canonical_path == *path,
        })
        .collect()
}

fn matching_descriptors<'a>(
    skills: &'a [Skill],
    selector: &SkillSelector,
) -> Vec<&'a SkillDescriptor> {
    matching_skills(skills, selector)
        .into_iter()
        .map(|skill| &skill.descriptor)
        .collect()
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: err.to_string(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}

fn json_result<T: serde::Serialize>(value: T) -> Result<serde_json::Value, JsonRpcError> {
    serde_json::to_value(value).map_err(internal_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::skills::{SkillActivationState, SkillExposure, SkillSource};

    fn skill(name: &str, path: &str) -> Skill {
        Skill {
            descriptor: SkillDescriptor {
                id: path.to_string(),
                name: name.to_string(),
                canonical_path: path.to_string(),
                source: SkillSource::BuiltIn,
                exposure: SkillExposure::DirectOnly,
                activation: SkillActivationState::Enabled,
                description: "test skill".to_string(),
                short_description: None,
                experimental: false,
                diagnostics: Vec::new(),
                agent_metadata: None,
            },
            body: "body".to_string(),
        }
    }

    #[test]
    fn skills_selector_matches_unique_name_or_path() {
        let skills = vec![skill("commit", "roder-builtin://commit/SKILL.md")];
        assert!(
            find_skill(
                &skills,
                &SkillSelector::Name {
                    name: "commit".into()
                }
            )
            .is_some()
        );
        assert!(
            find_skill(
                &skills,
                &SkillSelector::Path {
                    path: "roder-builtin://commit/SKILL.md".into()
                }
            )
            .is_some()
        );
    }

    #[test]
    fn skills_selector_refuses_ambiguous_names() {
        let skills = vec![
            skill("commit", "roder-builtin://commit/SKILL.md"),
            skill("commit", "workspace://.agents/skills/commit/SKILL.md"),
        ];
        assert!(
            find_skill(
                &skills,
                &SkillSelector::Name {
                    name: "commit".into()
                }
            )
            .is_none()
        );
    }

    #[test]
    fn skills_config_updates_preserve_existing_rule_fields() {
        let mut config = roder_skills::SkillsConfig::default();
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
