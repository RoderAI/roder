//! Resource enumeration: turns a package root + manifest + record into the
//! concrete [`PackageResource`] list, applying settings filters and
//! per-resource disables.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use roder_api::packages::{
    PackageManifestSpec, PackageRecord, PackageResource, PackageResourceKind, glob_match,
};
use roder_api::process_extension::ProcessExtensionManifest;

/// Enumerates the resources of one package. Soft problems (broken extension
/// manifests, declared entries that match nothing) come back as diagnostics.
pub fn enumerate_resources(
    root: &Path,
    spec: &PackageManifestSpec,
    record: &PackageRecord,
) -> (Vec<PackageResource>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let tree = walk_tree(root);
    let mut resources = Vec::new();

    enumerate_skills(spec, record, &tree, &mut resources, &mut diagnostics);
    enumerate_dir_files(
        PackageResourceKind::Command,
        &spec.commands,
        "md",
        record,
        &tree,
        &mut resources,
        &mut diagnostics,
    );
    enumerate_dir_files(
        PackageResourceKind::Theme,
        &spec.themes,
        "css",
        record,
        &tree,
        &mut resources,
        &mut diagnostics,
    );
    enumerate_extensions(root, spec, record, &tree, &mut resources, &mut diagnostics);

    resources.sort_by(|a, b| (a.kind, &a.path).cmp(&(b.kind, &b.path)));
    resources.dedup_by(|a, b| a.kind == b.kind && a.path == b.path);
    (resources, diagnostics)
}

/// Snapshot of the package tree (relative slash paths), skipping `.git` and
/// `node_modules` so dependency trees are never scanned for resources.
struct PackageTree {
    files: Vec<String>,
    dirs: Vec<String>,
}

fn walk_tree(root: &Path) -> PackageTree {
    let mut tree = PackageTree {
        files: Vec::new(),
        dirs: Vec::new(),
    };
    walk_into(root, String::new(), &mut tree);
    tree.files.sort();
    tree.dirs.sort();
    tree
}

fn walk_into(dir: &Path, rel: String, tree: &mut PackageTree) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let entry_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if file_type.is_dir() {
            if name == ".git" || name == "node_modules" {
                continue;
            }
            tree.dirs.push(entry_rel.clone());
            walk_into(&entry.path(), entry_rel, tree);
        } else if file_type.is_file() {
            tree.files.push(entry_rel);
        }
    }
}

fn normalize_entry(entry: &str) -> String {
    entry
        .trim()
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

/// Full glob match for a concrete path: the pattern must consume the whole
/// path, not just a leading directory prefix ([`glob_match`] treats prefix
/// matches as hits so filters can name directories).
fn matches_exactly(pattern: &str, rel: &str) -> bool {
    if !glob_match(pattern, rel) {
        return false;
    }
    match rel.rsplit_once('/') {
        Some((parent, _)) => !glob_match(pattern, parent),
        None => true,
    }
}

fn enumerate_skills(
    spec: &PackageManifestSpec,
    record: &PackageRecord,
    tree: &PackageTree,
    resources: &mut Vec<PackageResource>,
    diagnostics: &mut Vec<String>,
) {
    // Skill = any directory containing a SKILL.md (recursively below a
    // declared entry), named after the directory. A SKILL.md directly inside
    // the declared directory counts with that directory's name.
    let skill_dirs: Vec<&str> = tree
        .files
        .iter()
        .filter(|file| file.as_str() == "SKILL.md" || file.ends_with("/SKILL.md"))
        .map(|file| {
            file.rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("")
        })
        .collect();
    for raw_entry in &spec.skills {
        let entry = normalize_entry(raw_entry);
        if entry.is_empty() {
            continue;
        }
        let mut matched = false;
        let mut seen = BTreeSet::new();
        for dir in &skill_dirs {
            if dir.is_empty() || !glob_match(&entry, dir) || !seen.insert(dir.to_string()) {
                continue;
            }
            matched = true;
            let name = dir.rsplit('/').next().unwrap_or(dir).to_string();
            push_resource(
                PackageResourceKind::Skill,
                dir.to_string(),
                name,
                false,
                record,
                resources,
            );
        }
        if !matched {
            diagnostics.push(format!(
                "skills entry {raw_entry:?} matched no directories containing SKILL.md"
            ));
        }
    }
}

/// Commands and themes: files with the right extension directly inside the
/// declared directories (non-recursive, matching how command/theme
/// directories load elsewhere).
fn enumerate_dir_files(
    kind: PackageResourceKind,
    entries: &[String],
    extension: &str,
    record: &PackageRecord,
    tree: &PackageTree,
    resources: &mut Vec<PackageResource>,
    diagnostics: &mut Vec<String>,
) {
    let suffix = format!(".{extension}");
    for raw_entry in entries {
        let entry = normalize_entry(raw_entry);
        if entry.is_empty() {
            continue;
        }
        let dirs: Vec<&str> = if entry.contains('*') {
            tree.dirs
                .iter()
                .map(String::as_str)
                .filter(|dir| matches_exactly(&entry, dir))
                .collect()
        } else if tree.dirs.iter().any(|dir| dir == &entry) {
            vec![entry.as_str()]
        } else {
            Vec::new()
        };
        let mut matched = false;
        for dir in dirs {
            for file in &tree.files {
                let Some(name) = file
                    .strip_prefix(dir)
                    .and_then(|rest| rest.strip_prefix('/'))
                else {
                    continue;
                };
                if name.contains('/') || !name.ends_with(&suffix) {
                    continue;
                }
                matched = true;
                let stem = name.trim_end_matches(&suffix).to_string();
                push_resource(kind, file.clone(), stem, false, record, resources);
            }
        }
        if !matched {
            diagnostics.push(format!(
                "{kind} entry {raw_entry:?} matched no `.{extension}` files"
            ));
        }
    }
}

fn enumerate_extensions(
    root: &Path,
    spec: &PackageManifestSpec,
    record: &PackageRecord,
    tree: &PackageTree,
    resources: &mut Vec<PackageResource>,
    diagnostics: &mut Vec<String>,
) {
    for raw_entry in &spec.extensions {
        let entry = normalize_entry(raw_entry);
        if entry.is_empty() {
            continue;
        }
        let manifest_paths: Vec<String> = if entry.contains('*') {
            tree.files
                .iter()
                .filter(|file| matches_exactly(&entry, file))
                .cloned()
                .collect()
        } else if tree.files.iter().any(|file| file == &entry) {
            vec![entry.clone()]
        } else {
            Vec::new()
        };
        if manifest_paths.is_empty() {
            diagnostics.push(format!(
                "extensions entry {raw_entry:?} matched no manifest files"
            ));
            continue;
        }
        for rel in manifest_paths {
            let manifest_path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let manifest = match read_extension_manifest(&manifest_path) {
                Ok(manifest) => manifest,
                Err(err) => {
                    diagnostics.push(format!("skipped extension {rel}: {err:#}"));
                    continue;
                }
            };
            let launches = manifest
                .launch
                .as_ref()
                .is_some_and(|launch| !launch.command.trim().is_empty());
            if !launches {
                diagnostics.push(format!(
                    "skipped extension {rel}: package extensions must declare a `[launch]` \
                     section with a non-empty command"
                ));
                continue;
            }
            push_resource(
                PackageResourceKind::Extension,
                rel,
                manifest.id.clone(),
                true,
                record,
                resources,
            );
        }
    }
}

pub(crate) fn read_extension_manifest(path: &Path) -> anyhow::Result<ProcessExtensionManifest> {
    let text = fs::read_to_string(path)
        .map_err(|err| anyhow::anyhow!("read {}: {err}", path.display()))?;
    toml::from_str(&text).map_err(|err| anyhow::anyhow!("parse {}: {err}", path.display()))
}

fn push_resource(
    kind: PackageResourceKind,
    path: String,
    name: String,
    requires_approval: bool,
    record: &PackageRecord,
    resources: &mut Vec<PackageResource>,
) {
    // Filters narrow the manifest: filtered-out resources do not enumerate.
    if !record.filters.allows(kind, &path) {
        return;
    }
    let mut resource = PackageResource {
        package_id: record.package_id.clone(),
        kind,
        path,
        name,
        enabled: true,
        requires_approval,
    };
    resource.enabled = record.enabled && !record.disabled_resources.contains(&resource.id());
    resources.push(resource);
}
