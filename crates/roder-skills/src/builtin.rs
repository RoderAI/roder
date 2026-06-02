use roder_api::skills::{Skill, SkillExposure, SkillSource};

use crate::metadata::skill_from_markdown;

const VCS_SNAPSHOT_SKILL: &str = include_str!("../builtin/vcs-snapshot/SKILL.md");
const WEBWRIGHT_SKILL: &str = include_str!("../builtin/webwright/SKILL.md");

pub fn builtin_skills() -> Vec<Skill> {
    [
        ("vcs-snapshot", VCS_SNAPSHOT_SKILL),
        ("webwright", WEBWRIGHT_SKILL),
    ]
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
    fn builtin_registry_loads_vcs_snapshot_skill_asset() {
        let skills = builtin_skills();

        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].descriptor.name, "vcs-snapshot");
        assert_eq!(skills[0].descriptor.source, SkillSource::BuiltIn);
        assert_eq!(skills[0].descriptor.exposure, SkillExposure::DirectOnly);
        assert_eq!(
            skills[0].descriptor.canonical_path,
            "roder-builtin://vcs-snapshot/SKILL.md"
        );
        assert!(skills[0].body.contains("VCS status"));
        assert_eq!(skills[1].descriptor.name, "webwright");
        assert_eq!(skills[1].descriptor.source, SkillSource::BuiltIn);
        assert_eq!(skills[1].descriptor.exposure, SkillExposure::DirectOnly);
        assert_eq!(
            skills[1].descriptor.canonical_path,
            "roder-builtin://webwright/SKILL.md"
        );
        assert!(skills[1].body.contains("webwright.prepare_workspace"));
    }
}
