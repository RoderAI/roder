use roder_api::skills::{
    SkillActivationState, SkillDescriptor, SkillExposure, SkillSelector, SkillSource,
};

use super::{PaletteAction, PaletteItem, StaticPaletteSource};

pub fn skill_source(skills: &[SkillDescriptor]) -> StaticPaletteSource {
    let mut entries = Vec::new();
    for skill in skills {
        let enabled = skill.activation != SkillActivationState::Disabled;
        let selector = SkillSelector::Path {
            path: skill.canonical_path.clone(),
        };
        entries.push((
            PaletteItem {
                id: format!("skill-{}", skill.canonical_path),
                title: format!("Skill: {}", skill.name),
                subtitle: Some(format!(
                    "{} | {} | {} | {}",
                    source_label(&skill.source),
                    activation_label(skill.activation),
                    exposure_label(skill.exposure),
                    skill.canonical_path
                )),
                keywords: skill_keywords(skill),
                icon: Some('$'),
            },
            PaletteAction::InsertComposerText(format!("${} ", skill.name)),
        ));
        entries.push((
            PaletteItem {
                id: format!("skill-toggle-{}", skill.canonical_path),
                title: format!(
                    "{} skill: {}",
                    if enabled { "Disable" } else { "Enable" },
                    skill.name
                ),
                subtitle: Some(one_line(&skill.description)),
                keywords: vec![
                    "skills".to_string(),
                    "manager".to_string(),
                    skill.name.clone(),
                    (if enabled { "disable" } else { "enable" }).to_string(),
                ],
                icon: Some(if enabled { '-' } else { '+' }),
            },
            PaletteAction::SetSkillEnabled {
                selector: selector.clone(),
                enabled: !enabled,
            },
        ));
        entries.push((
            PaletteItem {
                id: format!("skill-exposure-{}", skill.canonical_path),
                title: format!(
                    "Set {} skill exposure: {}",
                    skill.name,
                    if skill.exposure == SkillExposure::Global {
                        "direct-only"
                    } else {
                        "global"
                    }
                ),
                subtitle: Some("Toggle global index visibility for this skill".to_string()),
                keywords: vec![
                    "skills".to_string(),
                    "manager".to_string(),
                    "exposure".to_string(),
                    skill.name.clone(),
                ],
                icon: Some('$'),
            },
            PaletteAction::SetSkillExposure {
                selector,
                exposure: if skill.exposure == SkillExposure::Global {
                    SkillExposure::DirectOnly
                } else {
                    SkillExposure::Global
                },
            },
        ));
    }
    StaticPaletteSource::new("skills", "Skills", entries)
}

fn skill_keywords(skill: &SkillDescriptor) -> Vec<String> {
    let mut keywords = vec![
        "skills".to_string(),
        "manager".to_string(),
        skill.name.clone(),
        source_label(&skill.source),
        activation_label(skill.activation).to_string(),
        exposure_label(skill.exposure).to_string(),
        skill.canonical_path.clone(),
    ];
    if let Some(short) = &skill.short_description {
        keywords.push(short.clone());
    }
    keywords
}

fn source_label(source: &SkillSource) -> String {
    match source {
        SkillSource::Workspace => "workspace".to_string(),
        SkillSource::User => "user".to_string(),
        SkillSource::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
        SkillSource::Imported { import_id } => format!("imported:{import_id}"),
        SkillSource::BuiltIn => "built-in".to_string(),
    }
}

fn activation_label(state: SkillActivationState) -> &'static str {
    match state {
        SkillActivationState::Enabled => "enabled",
        SkillActivationState::Disabled => "disabled",
        SkillActivationState::Experimental => "experimental",
    }
}

fn exposure_label(exposure: SkillExposure) -> &'static str {
    match exposure {
        SkillExposure::Global => "global",
        SkillExposure::DirectOnly => "direct-only",
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> SkillDescriptor {
        SkillDescriptor {
            id: "builtin:commit".to_string(),
            name: "commit".to_string(),
            canonical_path: "roder-builtin://commit/SKILL.md".to_string(),
            source: SkillSource::BuiltIn,
            exposure: SkillExposure::DirectOnly,
            activation: SkillActivationState::Enabled,
            description: "Commit staged changes safely.".to_string(),
            short_description: Some("Commit safely".to_string()),
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        }
    }

    #[test]
    fn skills_palette_exposes_manager_rows() {
        let source = skill_source(&[descriptor()]);
        let entries = source.entries();
        let titles = entries
            .iter()
            .map(|entry| entry.item.title.as_str())
            .collect::<Vec<_>>();
        assert!(titles.contains(&"Skill: commit"));
        assert!(titles.contains(&"Disable skill: commit"));
        assert!(titles.contains(&"Set commit skill exposure: global"));
    }
}
