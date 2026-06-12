//! Consumer-facing snapshot of enabled package resources, plus the
//! convenience views the skills registry, command loader, theme discovery,
//! and process-extension host consume.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use roder_api::packages::{
    PackageManifestSpec, PackageRecord, PackageResource, PackageResourceKind, PackageScope,
    PackageSource,
};
use roder_api::process_extension::{ProcessEventFilter, ProcessExtensionConfig};
use time::OffsetDateTime;

use super::manifest::load_package_manifest;
use super::ops::package_root;
use super::paths::PackagePaths;
use super::resources::{enumerate_resources, read_extension_manifest};
use super::settings::load_settings;

#[derive(Debug)]
pub struct PackageSnapshot {
    pub record: PackageRecord,
    /// Absolute package root resources load from.
    pub root: PathBuf,
    pub manifest: PackageManifestSpec,
    pub resources: Vec<PackageResource>,
}

/// Snapshot of every loadable package: project records shadow user records
/// by identity, disabled packages and missing roots are skipped, and
/// ephemeral roots from [`PackagePaths::ephemeral_roots`] are appended as
/// enabled project-scope local packages. Problems come back as diagnostics.
pub fn enabled_package_resources(paths: &PackagePaths) -> (Vec<PackageSnapshot>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let mut records: Vec<PackageRecord> = Vec::new();
    let mut seen_identities = HashSet::new();

    for scope in [PackageScope::Project, PackageScope::User] {
        if scope == PackageScope::Project && paths.workspace.is_none() {
            continue;
        }
        let settings_path = match paths.settings_path(scope) {
            Ok(path) => path,
            Err(err) => {
                diagnostics.push(format!("{err:#}"));
                continue;
            }
        };
        match load_settings(&settings_path) {
            Ok(settings) => {
                for record in settings.packages {
                    // Project entries load first and win on identity.
                    if seen_identities.insert(record.identity.clone()) {
                        records.push(record);
                    }
                }
            }
            Err(err) => diagnostics.push(format!("{err:#}")),
        }
    }

    let mut snapshots = Vec::new();
    for record in records {
        if !record.enabled {
            continue;
        }
        let Some(root) = package_root(&record) else {
            continue;
        };
        if !root.is_dir() {
            diagnostics.push(format!(
                "package {} root {} is missing; run `roder packages sync` or reinstall",
                record.package_id,
                root.display()
            ));
            continue;
        }
        match snapshot_package(record, &root) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(err) => diagnostics.push(format!("{err:#}")),
        }
    }

    for root in &paths.ephemeral_roots {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if !root.is_dir() {
            diagnostics.push(format!(
                "ephemeral package root {} does not exist",
                root.display()
            ));
            continue;
        }
        let source = PackageSource::LocalPath {
            path: root.display().to_string(),
        };
        if !seen_identities.insert(source.identity()) {
            continue;
        }
        match ephemeral_record(source, paths.ephemeral_extensions_approved) {
            Ok(record) => match snapshot_package(record, &root) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(err) => diagnostics.push(format!("{err:#}")),
            },
            Err(err) => diagnostics.push(format!("{err:#}")),
        }
    }

    (snapshots, diagnostics)
}

fn snapshot_package(record: PackageRecord, root: &Path) -> anyhow::Result<PackageSnapshot> {
    let (manifest, _manifest_diagnostics) = load_package_manifest(root, &record.source)?;
    let mut record = record;
    // Ephemeral records take their package id from the manifest; persisted
    // records keep the id they were installed under so disabled-resource ids
    // keep matching.
    if record.package_id.is_empty() {
        record.package_id = manifest.spec.id.clone();
    }
    let (resources, _resource_diagnostics) = enumerate_resources(root, &manifest.spec, &record);
    Ok(PackageSnapshot {
        root: root.to_path_buf(),
        manifest: manifest.spec,
        resources,
        record,
    })
}

/// Synthetic record for an ephemeral root: enabled, project-scope, never
/// persisted.
fn ephemeral_record(
    source: PackageSource,
    extensions_approved: bool,
) -> anyhow::Result<PackageRecord> {
    Ok(PackageRecord {
        package_id: String::new(), // filled from the manifest in snapshot_package
        identity: source.identity(),
        source,
        scope: PackageScope::Project,
        install_path: None,
        resolved: None,
        enabled: true,
        allow_scripts: false,
        extensions_approved,
        installed_at: OffsetDateTime::now_utc(),
        content_hash: None,
        filters: Default::default(),
        disabled_resources: Vec::new(),
    })
}

/// Skill roots for `build_skills_registry`: one entry per declared skill
/// directory that exists and contains at least one enabled skill resource.
/// Returns `(package_id, absolute dir, canonical prefix)` where the prefix
/// is `package://<id>/<rel>`. Glob skill entries have no stable root
/// directory and are skipped here (their skills still enumerate as
/// resources).
pub fn package_skill_roots(paths: &PackagePaths) -> Vec<(String, PathBuf, String)> {
    declared_dirs_with_enabled_resources(paths, PackageResourceKind::Skill)
        .into_iter()
        .map(|(package_id, abs, rel)| {
            let prefix = format!("package://{package_id}/{rel}");
            (package_id, abs, prefix)
        })
        .collect()
}

/// Declared command directories (absolute) containing at least one enabled
/// command resource, with their package id.
pub fn package_command_dirs(paths: &PackagePaths) -> Vec<(String, PathBuf)> {
    declared_dirs_with_enabled_resources(paths, PackageResourceKind::Command)
        .into_iter()
        .map(|(package_id, abs, _rel)| (package_id, abs))
        .collect()
}

/// Declared theme directories (absolute) containing at least one enabled
/// theme resource.
pub fn package_theme_dirs(paths: &PackagePaths) -> Vec<PathBuf> {
    declared_dirs_with_enabled_resources(paths, PackageResourceKind::Theme)
        .into_iter()
        .map(|(_package_id, abs, _rel)| abs)
        .collect()
}

fn declared_dirs_with_enabled_resources(
    paths: &PackagePaths,
    kind: PackageResourceKind,
) -> Vec<(String, PathBuf, String)> {
    let (snapshots, _diagnostics) = enabled_package_resources(paths);
    let mut dirs = Vec::new();
    for snapshot in &snapshots {
        let entries = match kind {
            PackageResourceKind::Skill => &snapshot.manifest.skills,
            PackageResourceKind::Command => &snapshot.manifest.commands,
            PackageResourceKind::Theme => &snapshot.manifest.themes,
            PackageResourceKind::Extension => continue,
        };
        for entry in entries {
            let entry = entry.trim().trim_start_matches("./").trim_matches('/');
            if entry.is_empty() || entry.contains('*') {
                continue;
            }
            let abs = snapshot.root.join(entry);
            if !abs.is_dir() {
                continue;
            }
            let has_enabled = snapshot.resources.iter().any(|resource| {
                resource.kind == kind
                    && resource.enabled
                    && (resource.path == entry
                        || resource
                            .path
                            .strip_prefix(entry)
                            .is_some_and(|rest| rest.starts_with('/')))
            });
            if has_enabled {
                dirs.push((snapshot.record.package_id.clone(), abs, entry.to_string()));
            }
        }
    }
    dirs
}

/// Launchable process-extension configs from packages whose extensions were
/// explicitly approved. Unapproved packages contribute nothing here — that
/// is the activation safety gate.
pub fn package_process_extensions(paths: &PackagePaths) -> Vec<ProcessExtensionConfig> {
    let (snapshots, _diagnostics) = enabled_package_resources(paths);
    let mut configs = Vec::new();
    for snapshot in &snapshots {
        if !snapshot.record.extensions_approved {
            continue;
        }
        for resource in &snapshot.resources {
            if resource.kind != PackageResourceKind::Extension || !resource.enabled {
                continue;
            }
            let manifest_path = snapshot
                .root
                .join(resource.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let Ok(manifest) = read_extension_manifest(&manifest_path) else {
                continue;
            };
            let Some(launch) = manifest.launch else {
                continue; // enumeration already skips these; belt and braces
            };
            let manifest_dir = manifest_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| snapshot.root.clone());
            // Relative cwd (and therefore relative paths in args) resolve
            // against the manifest's directory.
            let cwd = match &launch.cwd {
                Some(cwd) if Path::new(cwd).is_absolute() => PathBuf::from(cwd),
                Some(cwd) => manifest_dir.join(cwd),
                None => manifest_dir,
            };
            configs.push(ProcessExtensionConfig {
                id: manifest.id,
                enabled: true,
                manifest: manifest_path.display().to_string(),
                command: launch.command,
                args: launch.args,
                cwd: Some(cwd.display().to_string()),
                env: launch.env,
                startup_timeout_ms: launch.startup_timeout_ms.unwrap_or(10_000),
                event_filter: ProcessEventFilter {
                    kinds: launch.event_filter_kinds,
                },
            });
        }
    }
    configs
}
