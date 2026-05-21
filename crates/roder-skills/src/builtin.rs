use roder_api::skills::{Skill, SkillExposure, SkillSource};

use crate::metadata::skill_from_markdown;

const COMMIT_SKILL: &str = include_str!("../builtin/commit/SKILL.md");

pub fn builtin_skills() -> Vec<Skill> {
    [("commit", COMMIT_SKILL)]
        .into_iter()
        .filter_map(|(name, text)| {
            skill_from_markdown(
                text,
                SkillSource::BuiltIn,
                format!("roder-builtin://{name}/SKILL.md"),
                SkillExposure::DirectOnly,
                None,
            )
            .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::skills::{SkillExposure, SkillSource};

    #[test]
    fn builtin_registry_loads_commit_skill_asset() {
        let skills = builtin_skills();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].descriptor.name, "commit");
        assert_eq!(skills[0].descriptor.source, SkillSource::BuiltIn);
        assert_eq!(skills[0].descriptor.exposure, SkillExposure::DirectOnly);
        assert_eq!(
            skills[0].descriptor.canonical_path,
            "roder-builtin://commit/SKILL.md"
        );
        assert!(skills[0].body.contains("git status"));
    }
}
