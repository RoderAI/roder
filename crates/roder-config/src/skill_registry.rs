use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{marketplaces, packages};

pub fn build_skills_registry(
    workspace: impl AsRef<Path>,
    skills: Option<&roder_skills::SkillsConfig>,
) -> roder_skills::SkillRegistry {
    let workspace = workspace.as_ref();
    let mut options = roder_skills::SkillRegistryOptions::new(workspace);
    options.config_rules = skills
        .map(|skills| skills.config.clone())
        .unwrap_or_default();
    for (root, canonical_prefix) in local_skill_roots(workspace) {
        options
            .roots
            .push(roder_skills::SkillRoot::workspace(root, canonical_prefix));
    }
    add_user_skill_roots(&mut options, dirs::home_dir().as_deref());
    if let Ok(store) = marketplaces::load_marketplace_store() {
        for plugin in store.installed_plugins.iter().filter(|plugin| {
            plugin.state == roder_api::marketplace::MarketplaceInstallState::Installed
        }) {
            options.roots.push(roder_skills::SkillRoot::plugin(
                plugin.variant_key.clone(),
                PathBuf::from(&plugin.install_path).join("skills"),
                format!("plugin://{}/skills", plugin.variant_key),
            ));
        }
    }
    let package_paths = packages::PackagePaths::standard(Some(workspace));
    for (package_id, root, canonical_prefix) in packages::package_skill_roots(&package_paths) {
        options.roots.push(roder_skills::SkillRoot::plugin(
            format!("pkg-{package_id}"),
            root,
            canonical_prefix,
        ));
    }
    roder_skills::SkillRegistry::load(options)
}

fn local_skill_roots(workspace: &Path) -> Vec<(PathBuf, String)> {
    let workspace = absolute_path(workspace);
    let project_root = project_root_for_skill_scan(&workspace);
    let mut bases = Vec::new();
    let mut current = workspace.as_path();
    loop {
        bases.push(current.to_path_buf());
        if current == project_root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    bases.reverse();

    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for base in bases {
        for relative in [
            ".agents/skills",
            ".claude/skills",
            ".cursor/skills",
            ".cursor/rules",
        ] {
            let root = base.join(relative);
            if !root.exists() && base != workspace {
                continue;
            }
            let key = root.clone();
            if !seen.insert(key) {
                continue;
            }
            let canonical_prefix =
                format!("workspace://{}", relative_path_string(&project_root, &root));
            roots.push((root, canonical_prefix));
        }
    }
    roots
}

fn project_root_for_skill_scan(workspace: &Path) -> PathBuf {
    let mut cargo_root = None;
    for ancestor in workspace.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
        if ancestor.join("Cargo.toml").exists() {
            cargo_root = Some(ancestor.to_path_buf());
        }
    }
    cargo_root.unwrap_or_else(|| workspace.to_path_buf())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn relative_path_string(from: &Path, to: &Path) -> String {
    if let Ok(relative) = to.strip_prefix(from) {
        return path_to_slash_string(relative);
    }

    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return path_to_slash_string(to);
    }

    let mut relative = PathBuf::new();
    for _ in common..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[common..] {
        relative.push(component.as_os_str());
    }
    path_to_slash_string(&relative)
}

fn path_to_slash_string(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

fn add_user_skill_roots(options: &mut roder_skills::SkillRegistryOptions, home: Option<&Path>) {
    if let Some(home) = home {
        for relative in [".agents/skills", ".codex/skills"] {
            options.roots.push(roder_skills::SkillRoot::user(
                home.join(relative),
                format!("user://{relative}"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_user_skills_load_with_canonical_paths_and_config_rules() {
        let root = std::env::temp_dir().join(format!("roder-user-skills-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        for (relative, name) in [(".agents/skills", "shared"), (".codex/skills", "codex")] {
            let path = home.join(relative).join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: Test skill\n---\nTest guidance.\n"),
            )
            .unwrap();
        }
        let mut options = roder_skills::SkillRegistryOptions::new(&workspace);
        add_user_skill_roots(&mut options, Some(&home));
        options.config_rules = vec![roder_skills::SkillConfigRule {
            path: Some("user://.agents/skills/shared/SKILL.md".to_string()),
            enabled: Some(false),
            name: None,
            exposure: None,
        }];
        let registry = roder_skills::SkillRegistry::load(options);
        assert!(
            registry.diagnostics().is_empty(),
            "{:?}",
            registry.diagnostics()
        );
        for (name, relative) in [("shared", ".agents/skills"), ("codex", ".codex/skills")] {
            let skill = registry
                .skills()
                .iter()
                .find(|skill| skill.descriptor.name == name)
                .unwrap();
            assert_eq!(
                skill.descriptor.canonical_path,
                format!("user://{relative}/{name}/SKILL.md")
            );
            assert_eq!(
                skill.descriptor.activation == roder_api::skills::SkillActivationState::Disabled,
                name == "shared"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_home_adds_no_user_roots() {
        let mut options = roder_skills::SkillRegistryOptions::new(".");
        add_user_skill_roots(&mut options, None);
        assert!(options.roots.is_empty());
    }
}
