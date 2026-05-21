use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roder_api::skills::{Skill, SkillActivationState, SkillExposure, SkillSelector, SkillSource};
use roder_api::workflow::{WorkflowImportItem, WorkflowImportState, WorkflowSourceType};

use crate::builtin::builtin_skills;
use crate::config::{SkillConfigRule, apply_skill_config};
use crate::metadata::load_skill_from_paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoot {
    pub root: PathBuf,
    pub source: SkillSource,
    pub canonical_prefix: String,
}

impl SkillRoot {
    pub fn workspace(root: impl Into<PathBuf>, canonical_prefix: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            source: SkillSource::Workspace,
            canonical_prefix: canonical_prefix.into(),
        }
    }

    pub fn user(root: impl Into<PathBuf>, canonical_prefix: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            source: SkillSource::User,
            canonical_prefix: canonical_prefix.into(),
        }
    }

    pub fn plugin(
        plugin_id: impl Into<String>,
        root: impl Into<PathBuf>,
        canonical_prefix: impl Into<String>,
    ) -> Self {
        Self {
            root: root.into(),
            source: SkillSource::Plugin {
                plugin_id: plugin_id.into(),
            },
            canonical_prefix: canonical_prefix.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkillRegistryOptions {
    pub workspace: PathBuf,
    pub include_builtins: bool,
    pub roots: Vec<SkillRoot>,
    pub workflow_imports: Vec<WorkflowImportItem>,
    pub config_rules: Vec<SkillConfigRule>,
}

impl SkillRegistryOptions {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            include_builtins: true,
            roots: Vec::new(),
            workflow_imports: Vec::new(),
            config_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
    diagnostics: Vec<String>,
}

impl SkillRegistry {
    pub fn load(options: SkillRegistryOptions) -> Self {
        let mut registry = Self::default();
        if options.include_builtins {
            registry.skills.extend(builtin_skills());
        }
        for root in &options.roots {
            registry.load_root(root);
        }
        registry.load_workflow_imports(&options.workspace, &options.workflow_imports);
        registry.apply_config(&options.config_rules);
        registry.sort();
        registry
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn resolve(&self, selector: &SkillSelector) -> Result<&Skill, SkillResolutionError> {
        let matches: Vec<&Skill> = self
            .skills
            .iter()
            .filter(|skill| selector_matches(selector, skill))
            .collect();
        match matches.as_slice() {
            [] => Err(SkillResolutionError::Missing(selector.clone())),
            [skill] if skill.descriptor.activation == SkillActivationState::Disabled => Err(
                SkillResolutionError::Disabled(skill.descriptor.canonical_path.clone()),
            ),
            [skill] => Ok(skill),
            skills => Err(SkillResolutionError::Ambiguous {
                name: selector_name(selector).unwrap_or_default(),
                canonical_paths: skills
                    .iter()
                    .map(|skill| skill.descriptor.canonical_path.clone())
                    .collect(),
            }),
        }
    }

    fn load_root(&mut self, root: &SkillRoot) {
        let Ok(entries) = std::fs::read_dir(&root.root) else {
            return;
        };
        for entry in entries.flatten() {
            let skill_path = entry.path().join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let canonical_path = format!("{}/{name}/SKILL.md", root.canonical_prefix);
            match load_skill_from_paths(
                &skill_path,
                root.source.clone(),
                canonical_path,
                SkillExposure::Global,
            ) {
                Ok(skill) => self.skills.push(skill),
                Err(err) => self
                    .diagnostics
                    .push(format!("{}: {err}", skill_path.display())),
            }
        }
    }

    fn load_workflow_imports(&mut self, workspace: &Path, items: &[WorkflowImportItem]) {
        for item in items.iter().filter(|item| {
            item.state == WorkflowImportState::Enabled
                && item.source.source_type == WorkflowSourceType::Skill
        }) {
            let path = resolve_workflow_path(workspace, &item.source.path);
            let canonical_path = format!("workflow-import://{}/SKILL.md", item.id);
            match load_skill_from_paths(
                &path,
                SkillSource::Imported {
                    import_id: item.id.clone(),
                },
                canonical_path,
                SkillExposure::Global,
            ) {
                Ok(skill) => self.skills.push(skill),
                Err(err) => self.diagnostics.push(format!("{}: {err}", item.id)),
            }
        }
    }

    fn apply_config(&mut self, rules: &[SkillConfigRule]) {
        for skill in &mut self.skills {
            let applied = apply_skill_config(&skill.descriptor, rules);
            skill.descriptor.activation = applied.activation;
            skill.descriptor.exposure = applied.exposure;
            skill.descriptor.diagnostics.extend(applied.diagnostics);
        }
    }

    fn sort(&mut self) {
        self.skills.sort_by(|left, right| {
            source_priority(&left.descriptor.source)
                .cmp(&source_priority(&right.descriptor.source))
                .then(left.descriptor.name.cmp(&right.descriptor.name))
                .then(
                    left.descriptor
                        .canonical_path
                        .cmp(&right.descriptor.canonical_path),
                )
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillResolutionError {
    Missing(SkillSelector),
    Disabled(String),
    Ambiguous {
        name: String,
        canonical_paths: Vec<String>,
    },
}

pub fn duplicate_skill_names(skills: &[Skill]) -> BTreeMap<String, Vec<String>> {
    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for skill in skills {
        grouped
            .entry(skill.descriptor.name.clone())
            .or_default()
            .push(skill.descriptor.canonical_path.clone());
    }
    grouped.retain(|_, paths| paths.len() > 1);
    grouped
}

fn selector_matches(selector: &SkillSelector, skill: &Skill) -> bool {
    match selector {
        SkillSelector::Name { name } => &skill.descriptor.name == name,
        SkillSelector::Path { path } => &skill.descriptor.canonical_path == path,
    }
}

fn selector_name(selector: &SkillSelector) -> Option<String> {
    match selector {
        SkillSelector::Name { name } => Some(name.clone()),
        SkillSelector::Path { .. } => None,
    }
}

fn resolve_workflow_path(workspace: &Path, source_path: &str) -> PathBuf {
    let path = PathBuf::from(source_path);
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

fn source_priority(source: &SkillSource) -> u8 {
    match source {
        SkillSource::BuiltIn => 0,
        SkillSource::Workspace => 1,
        SkillSource::User => 2,
        SkillSource::Plugin { .. } => 3,
        SkillSource::Imported { .. } => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::workflow::{
        WorkflowImportItem, WorkflowImportRisk, WorkflowSource, WorkflowSourceType,
    };
    use time::OffsetDateTime;

    #[test]
    fn registry_loads_builtin_workspace_user_plugin_and_imported_skills() {
        let workspace = fixture_dir("all-sources");
        write_skill(
            &workspace.join(".agents/skills/review"),
            "review",
            "Workspace review",
        );
        let user_root = workspace.join("user/skills");
        write_skill(&user_root.join("commit"), "commit", "User commit");
        let plugin_root = workspace.join("plugin/skills");
        write_skill(&plugin_root.join("lint"), "lint", "Plugin lint");
        let imported = workspace.join("imported/SKILL.md");
        std::fs::create_dir_all(imported.parent().unwrap()).unwrap();
        std::fs::write(
            &imported,
            "---\nname: imported-review\ndescription: Imported review\n---\nBody\n",
        )
        .unwrap();

        let registry = SkillRegistry::load(SkillRegistryOptions {
            workspace: workspace.clone(),
            include_builtins: true,
            roots: vec![
                SkillRoot::workspace(
                    workspace.join(".agents/skills"),
                    "workspace://.agents/skills",
                ),
                SkillRoot::user(&user_root, "user://skills"),
                SkillRoot::plugin("plugin-a", &plugin_root, "plugin://plugin-a/skills"),
            ],
            workflow_imports: vec![workflow_skill_item("imported", "imported/SKILL.md")],
            config_rules: Vec::new(),
        });

        assert!(registry.diagnostics().is_empty());
        assert!(registry.skills().iter().any(|skill| {
            skill.descriptor.source == SkillSource::BuiltIn && skill.descriptor.name == "commit"
        }));
        assert!(registry.skills().iter().any(|skill| {
            skill.descriptor.source == SkillSource::Workspace && skill.descriptor.name == "review"
        }));
        assert!(registry.skills().iter().any(|skill| {
            matches!(skill.descriptor.source, SkillSource::Plugin { .. })
                && skill.descriptor.name == "lint"
        }));
        assert!(registry.skills().iter().any(|skill| {
            matches!(skill.descriptor.source, SkillSource::Imported { .. })
                && skill.descriptor.name == "imported-review"
        }));
    }

    #[test]
    fn registry_preserves_duplicate_names_and_requires_path_selection() {
        let workspace = fixture_dir("duplicates");
        write_skill(
            &workspace.join(".agents/skills/review"),
            "review",
            "Workspace review",
        );
        let user_root = workspace.join("user/skills");
        write_skill(&user_root.join("review"), "review", "User review");

        let registry = SkillRegistry::load(SkillRegistryOptions {
            workspace: workspace.clone(),
            include_builtins: false,
            roots: vec![
                SkillRoot::workspace(
                    workspace.join(".agents/skills"),
                    "workspace://.agents/skills",
                ),
                SkillRoot::user(&user_root, "user://skills"),
            ],
            workflow_imports: Vec::new(),
            config_rules: Vec::new(),
        });

        assert_eq!(duplicate_skill_names(registry.skills())["review"].len(), 2);
        assert!(matches!(
            registry.resolve(&SkillSelector::Name {
                name: "review".to_string()
            }),
            Err(SkillResolutionError::Ambiguous { .. })
        ));
        let selected = registry
            .resolve(&SkillSelector::Path {
                path: "workspace://.agents/skills/review/SKILL.md".to_string(),
            })
            .unwrap();
        assert_eq!(selected.descriptor.source, SkillSource::Workspace);
    }

    #[test]
    fn registry_applies_config_and_reports_malformed_skills() {
        let workspace = fixture_dir("config");
        write_skill(
            &workspace.join(".agents/skills/review"),
            "review",
            "Workspace review",
        );
        std::fs::create_dir_all(workspace.join(".agents/skills/bad")).unwrap();
        std::fs::write(
            workspace.join(".agents/skills/bad/SKILL.md"),
            "no frontmatter",
        )
        .unwrap();

        let registry = SkillRegistry::load(SkillRegistryOptions {
            workspace: workspace.clone(),
            include_builtins: true,
            roots: vec![SkillRoot::workspace(
                workspace.join(".agents/skills"),
                "workspace://.agents/skills",
            )],
            workflow_imports: Vec::new(),
            config_rules: vec![SkillConfigRule {
                name: Some("commit".to_string()),
                path: None,
                enabled: Some(false),
                exposure: Some(SkillExposure::Global),
            }],
        });

        let commit = registry
            .resolve(&SkillSelector::Path {
                path: "roder-builtin://commit/SKILL.md".to_string(),
            })
            .unwrap_err();
        assert!(matches!(commit, SkillResolutionError::Disabled(_)));
        assert!(
            registry
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.contains("frontmatter"))
        );
    }

    fn write_skill(dir: &Path, name: &str, description: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nBody for {name}\n"),
        )
        .unwrap();
    }

    fn workflow_skill_item(id: &str, path: &str) -> WorkflowImportItem {
        WorkflowImportItem {
            id: id.to_string(),
            title: id.to_string(),
            summary: "summary".to_string(),
            source: WorkflowSource {
                source_type: WorkflowSourceType::Skill,
                path: path.to_string(),
                name: Some(id.to_string()),
                hash: "hash".to_string(),
                detected_at: OffsetDateTime::UNIX_EPOCH,
            },
            state: WorkflowImportState::Enabled,
            risk: WorkflowImportRisk::Passive,
            command_capable: false,
            approval_required: false,
            preview: serde_json::Value::Null,
            conflicts: Vec::new(),
            enabled_at: Some(OffsetDateTime::UNIX_EPOCH),
        }
    }

    fn fixture_dir(name: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("roder-skills-{name}-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
