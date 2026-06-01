use std::path::{Path, PathBuf};

use roder_api::skills::{SkillActivationState, SkillDescriptor, SkillExposure, SkillSource};
use roder_skills::{SkillConfigRule, apply_skill_config, load_skill_from_paths};

fn fixture(path: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/model")).join(path)
}

#[test]
fn model_loads_workspace_and_builtin_skill_metadata() {
    let workspace_path = fixture("workspace-review/SKILL.md");
    let workspace = load_skill_from_paths(
        &workspace_path,
        SkillSource::Workspace,
        "workspace://.agents/skills/review/SKILL.md".to_string(),
        SkillExposure::Global,
    )
    .unwrap();
    assert_eq!(workspace.descriptor.name, "review");
    assert_eq!(
        workspace.descriptor.short_description.as_deref(),
        Some("Code review")
    );
    assert_eq!(
        workspace
            .descriptor
            .agent_metadata
            .as_ref()
            .unwrap()
            .dependencies,
        vec!["git"]
    );

    let builtin_path = fixture("builtin-vcs-snapshot/SKILL.md");
    let builtin = load_skill_from_paths(
        &builtin_path,
        SkillSource::BuiltIn,
        "roder-builtin://vcs-snapshot/SKILL.md".to_string(),
        SkillExposure::DirectOnly,
    )
    .unwrap();
    assert_eq!(builtin.descriptor.source, SkillSource::BuiltIn);
    assert_eq!(builtin.descriptor.exposure, SkillExposure::DirectOnly);
    assert!(builtin.body.contains("stage only requested files"));
}

#[test]
fn model_serializes_all_skill_sources() {
    let descriptors = vec![
        descriptor("workspace", SkillSource::Workspace),
        descriptor("user", SkillSource::User),
        descriptor(
            "plugin",
            SkillSource::Plugin {
                plugin_id: "codex-plugins/review".to_string(),
            },
        ),
        descriptor(
            "imported",
            SkillSource::Imported {
                import_id: "workflow-import-review".to_string(),
            },
        ),
        descriptor("builtin", SkillSource::BuiltIn),
    ];

    let value = serde_json::to_value(&descriptors).unwrap();
    let round_trip: Vec<SkillDescriptor> = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip, descriptors);
}

#[test]
fn model_config_can_disable_builtin_and_change_exposure() {
    let mut descriptor = descriptor("vcs-snapshot", SkillSource::BuiltIn);
    descriptor.name = "vcs-snapshot".to_string();
    descriptor.canonical_path = "roder-builtin://vcs-snapshot/SKILL.md".to_string();
    descriptor.exposure = SkillExposure::DirectOnly;

    let applied = apply_skill_config(
        &descriptor,
        &[SkillConfigRule {
            name: Some("vcs-snapshot".to_string()),
            path: None,
            enabled: Some(false),
            exposure: Some(SkillExposure::Global),
        }],
    );

    assert_eq!(applied.activation, SkillActivationState::Disabled);
    assert_eq!(applied.exposure, SkillExposure::Global);
    assert!(applied.diagnostics[0].contains("built-in skill disabled"));
}

fn descriptor(name: &str, source: SkillSource) -> SkillDescriptor {
    SkillDescriptor {
        id: format!("skill:{name}"),
        name: name.to_string(),
        canonical_path: format!("test://{name}/SKILL.md"),
        source,
        exposure: SkillExposure::Global,
        activation: SkillActivationState::Enabled,
        description: format!("{name} skill"),
        short_description: Some(name.to_string()),
        experimental: false,
        diagnostics: Vec::new(),
        agent_metadata: None,
    }
}
