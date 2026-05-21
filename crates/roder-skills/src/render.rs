use roder_api::context::{ContextBlock, ContextBlockKind};
use roder_api::skills::{Skill, SkillActivationState, SkillExposure};

pub fn render_global_skill_index(skills: &[Skill]) -> Option<ContextBlock> {
    let mut visible = skills
        .iter()
        .filter(|skill| {
            skill.descriptor.activation == SkillActivationState::Enabled
                && skill.descriptor.exposure == SkillExposure::Global
        })
        .collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        left.descriptor.name.cmp(&right.descriptor.name).then(
            left.descriptor
                .canonical_path
                .cmp(&right.descriptor.canonical_path),
        )
    });
    if visible.is_empty() {
        return None;
    }
    let mut text = String::from("<skills>\n");
    for skill in visible {
        let short = skill
            .descriptor
            .short_description
            .as_deref()
            .unwrap_or(&skill.descriptor.description);
        text.push_str(&format!(
            "<skill name=\"{}\" path=\"{}\">\n<description>{}</description>\n<usage>Invoke with ${} or ${{{}}}.</usage>\n</skill>\n",
            escape_attr(&skill.descriptor.name),
            escape_attr(&skill.descriptor.canonical_path),
            escape_text(short),
            skill.descriptor.name,
            skill.descriptor.name
        ));
    }
    text.push_str("</skills>");
    Some(ContextBlock {
        id: "skills.global_index".to_string(),
        kind: ContextBlockKind::Instruction,
        priority: 80,
        token_estimate: Some(estimate_text_tokens(&text)),
        metadata: serde_json::json!({ "kind": "skills.global_index" }),
        text,
    })
}

pub fn render_skill_body(skill: &Skill) -> ContextBlock {
    let text = format!(
        "<skill name=\"{}\" path=\"{}\">\n{}\n</skill>",
        escape_attr(&skill.descriptor.name),
        escape_attr(&skill.descriptor.canonical_path),
        skill.body.trim()
    );
    ContextBlock {
        id: format!("skills.body.{}", skill.descriptor.id),
        kind: ContextBlockKind::Instruction,
        priority: 90,
        token_estimate: Some(estimate_text_tokens(&text)),
        metadata: serde_json::json!({
            "kind": "skills.body",
            "name": skill.descriptor.name,
            "canonicalPath": skill.descriptor.canonical_path,
        }),
        text,
    }
}

pub fn estimate_text_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::skills::{SkillDescriptor, SkillSource};

    #[test]
    fn render_global_index_excludes_direct_only_and_disabled_skills() {
        let index = render_global_skill_index(&[
            skill(
                "review",
                SkillExposure::Global,
                SkillActivationState::Enabled,
            ),
            skill(
                "commit",
                SkillExposure::DirectOnly,
                SkillActivationState::Enabled,
            ),
            skill("bad", SkillExposure::Global, SkillActivationState::Disabled),
        ])
        .unwrap();

        assert!(index.text.contains("review"));
        assert!(!index.text.contains("commit"));
        assert!(!index.text.contains("bad"));
    }

    #[test]
    fn render_skill_body_wraps_in_hidden_skill_envelope() {
        let block = render_skill_body(&skill(
            "commit",
            SkillExposure::DirectOnly,
            SkillActivationState::Enabled,
        ));

        assert!(block.text.starts_with("<skill name=\"commit\""));
        assert!(block.text.contains("Body for commit"));
        assert_eq!(block.kind, ContextBlockKind::Instruction);
    }

    fn skill(name: &str, exposure: SkillExposure, activation: SkillActivationState) -> Skill {
        Skill {
            descriptor: SkillDescriptor {
                id: format!("skill:{name}"),
                name: name.to_string(),
                canonical_path: format!("test://{name}/SKILL.md"),
                source: SkillSource::Workspace,
                exposure,
                activation,
                description: format!("{name} description"),
                short_description: None,
                experimental: false,
                diagnostics: Vec::new(),
                agent_metadata: None,
            },
            body: format!("Body for {name}"),
        }
    }
}
