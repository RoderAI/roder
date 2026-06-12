//! Core package lifecycle operations: install, remove, list, and record
//! toggles (update/sync live in [`super::update`]). All operations take a
//! [`PackagePaths`] so the whole layer is testable against temp directories.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::packages::{
    PackageIdentity, PackageRecord, PackageResource, PackageResourceFilters, PackageScope,
    PackageSource, parse_package_resource_id, parse_package_spec,
};
use time::OffsetDateTime;

use super::fsutil::content_hash;
use super::git::git_fetch_into_store;
use super::manifest::load_package_manifest;
use super::npm::{DEFAULT_NPM_COMMAND, npm_fetch_into_store};
use super::paths::PackagePaths;
use super::resources::enumerate_resources;
use super::settings::{load_settings, record_matches, save_settings};

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Allow npm lifecycle scripts to run during install. Recorded on the
    /// package record.
    pub allow_scripts: bool,
    /// Per-type resource filters layered on top of the manifest.
    pub filters: PackageResourceFilters,
    /// Base directory for resolving relative local-path specs. Defaults to
    /// the current working directory.
    pub resolve_base: Option<PathBuf>,
    /// npm invocation override. Defaults to `[packages] npm_command` from
    /// config.toml, then plain `npm`.
    pub npm_command: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct InstalledPackage {
    pub record: PackageRecord,
    pub resources: Vec<PackageResource>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug)]
pub struct ListedPackage {
    pub record: PackageRecord,
    /// True for a user-scope record whose identity is also installed in the
    /// project scope (the project entry wins for this workspace).
    pub shadowed_by_project: bool,
}

/// Installs (or idempotently re-installs) a package into a scope.
pub fn install_package(
    paths: &PackagePaths,
    scope: PackageScope,
    spec_str: &str,
    options: InstallOptions,
) -> anyhow::Result<InstalledPackage> {
    let source = resolve_spec_source(spec_str, options.resolve_base.as_deref())?;
    let identity = source.identity();
    let settings_path = paths.settings_path(scope)?;
    let mut settings = load_settings(&settings_path)?;
    let existing = settings
        .packages
        .iter()
        .find(|record| record.identity == identity)
        .cloned();

    let npm_command = resolve_npm_command(options.npm_command.clone());
    let mut diagnostics = Vec::new();
    let (root, install_path, resolved) = match &source {
        PackageSource::Npm { name, version } => {
            let store = paths
                .store_path(scope, &source)?
                .expect("npm sources always have a store path");
            let outcome = npm_fetch_into_store(
                &npm_command,
                name,
                version.as_deref(),
                options.allow_scripts,
                &store,
            )?;
            diagnostics.extend(outcome.warnings);
            (store.clone(), Some(store), outcome.resolved_version)
        }
        PackageSource::Git { url, ref_name } => {
            let store = paths
                .store_path(scope, &source)?
                .expect("git sources always have a store path");
            let outcome = git_fetch_into_store(
                url,
                ref_name.as_deref(),
                &store,
                &npm_command,
                options.allow_scripts,
            )?;
            diagnostics.extend(outcome.warnings);
            (store.clone(), Some(store), outcome.resolved_commit)
        }
        PackageSource::LocalPath { path } => {
            let root = PathBuf::from(path);
            anyhow::ensure!(
                root.is_dir(),
                "local package path {} does not exist or is not a directory",
                root.display()
            );
            // Local paths load in place: no store copy, install_path = None.
            (root, None, None)
        }
    };

    let (manifest, manifest_diagnostics) = load_package_manifest(&root, &source)?;
    diagnostics.extend(manifest_diagnostics);
    let package_id = manifest.spec.id.clone();

    if let Some(conflict) = settings
        .packages
        .iter()
        .find(|record| record.package_id == package_id && record.identity != identity)
    {
        anyhow::bail!(
            "package id {package_id:?} is already used by {} in {scope} scope; remove it first \
             with `roder remove {}`",
            conflict.identity,
            conflict.source.spec()
        );
    }

    let content_hash = content_hash(&root)
        .with_context(|| format!("hash package contents at {}", root.display()))?;
    let record = PackageRecord {
        package_id,
        identity,
        source,
        scope,
        install_path: install_path.map(|path| path.display().to_string()),
        resolved,
        // Re-install preserves user decisions that the install args do not
        // express: enabled state, per-resource disables, and extension
        // approval. allow_scripts and filters come from the install args.
        enabled: existing.as_ref().map(|e| e.enabled).unwrap_or(true),
        allow_scripts: options.allow_scripts,
        extensions_approved: existing
            .as_ref()
            .map(|e| e.extensions_approved)
            .unwrap_or(false),
        installed_at: OffsetDateTime::now_utc(),
        content_hash: Some(content_hash),
        filters: options.filters,
        disabled_resources: existing.map(|e| e.disabled_resources).unwrap_or_default(),
    };
    settings.upsert(record.clone());
    save_settings(&settings_path, &settings)?;

    let (resources, resource_diagnostics) = enumerate_resources(&root, &manifest.spec, &record);
    diagnostics.extend(resource_diagnostics);
    Ok(InstalledPackage {
        record,
        resources,
        diagnostics,
    })
}

/// Removes a package record and deletes its store directory. Store deletion
/// only happens inside the scope's packages store root; local-path sources
/// are never deleted.
pub fn remove_package(
    paths: &PackagePaths,
    scope: PackageScope,
    spec_or_id: &str,
) -> anyhow::Result<PackageRecord> {
    let settings_path = paths.settings_path(scope)?;
    let mut settings = load_settings(&settings_path)?;
    let parsed_identity = parsed_query_identity(spec_or_id);
    let index = settings
        .packages
        .iter()
        .position(|record| record_matches(record, spec_or_id, parsed_identity.as_ref()))
        .ok_or_else(|| {
            anyhow::anyhow!("package {spec_or_id:?} is not installed in {scope} scope")
        })?;
    let record = settings.packages.remove(index);
    save_settings(&settings_path, &settings)?;

    if let Some(install_path) = &record.install_path {
        let install_path = PathBuf::from(install_path);
        let store_root = paths.store_root(scope)?;
        if install_path.starts_with(&store_root) && install_path.exists() {
            fs::remove_dir_all(&install_path)
                .with_context(|| format!("delete package store {}", install_path.display()))?;
        }
    }
    Ok(record)
}

/// All records from both scopes; user records are flagged when a project
/// record with the same identity shadows them.
pub fn list_packages(paths: &PackagePaths) -> anyhow::Result<Vec<ListedPackage>> {
    let mut listed = Vec::new();
    let mut project_identities = HashSet::new();
    if paths.workspace.is_some() {
        let settings = load_settings(&paths.settings_path(PackageScope::Project)?)?;
        for record in settings.packages {
            project_identities.insert(record.identity.clone());
            listed.push(ListedPackage {
                record,
                shadowed_by_project: false,
            });
        }
    }
    let user = load_settings(&paths.settings_path(PackageScope::User)?)?;
    for record in user.packages {
        let shadowed_by_project = project_identities.contains(&record.identity);
        listed.push(ListedPackage {
            record,
            shadowed_by_project,
        });
    }
    Ok(listed)
}

pub fn set_package_enabled(
    paths: &PackagePaths,
    spec_or_id: &str,
    enabled: bool,
) -> anyhow::Result<PackageRecord> {
    with_record_mut(paths, spec_or_id, |record| record.enabled = enabled)
}

pub fn approve_extensions(
    paths: &PackagePaths,
    spec_or_id: &str,
    approved: bool,
) -> anyhow::Result<PackageRecord> {
    with_record_mut(paths, spec_or_id, |record| {
        record.extensions_approved = approved;
    })
}

pub fn set_filters(
    paths: &PackagePaths,
    spec_or_id: &str,
    filters: PackageResourceFilters,
) -> anyhow::Result<PackageRecord> {
    with_record_mut(paths, spec_or_id, |record| record.filters = filters)
}

/// Toggles one resource by id (`<package-id>:<kind>/<name>`).
pub fn set_resource_enabled(
    paths: &PackagePaths,
    resource_id: &str,
    enabled: bool,
) -> anyhow::Result<PackageRecord> {
    let (package_id, _kind, _name) = parse_package_resource_id(resource_id)?;
    let resource_id = resource_id.to_string();
    with_record_mut(paths, &package_id, move |record| {
        if enabled {
            record.disabled_resources.retain(|id| id != &resource_id);
        } else if !record.disabled_resources.contains(&resource_id) {
            record.disabled_resources.push(resource_id);
            record.disabled_resources.sort();
        }
    })
}

/// Finds a record by spec-or-id (project scope first, mirroring shadowing),
/// applies `mutate`, and persists.
fn with_record_mut(
    paths: &PackagePaths,
    spec_or_id: &str,
    mutate: impl FnOnce(&mut PackageRecord),
) -> anyhow::Result<PackageRecord> {
    let parsed_identity = parsed_query_identity(spec_or_id);
    for scope in [PackageScope::Project, PackageScope::User] {
        if scope == PackageScope::Project && paths.workspace.is_none() {
            continue;
        }
        let settings_path = paths.settings_path(scope)?;
        let mut settings = load_settings(&settings_path)?;
        if let Some(record) = settings.find_mut(spec_or_id, parsed_identity.as_ref()) {
            mutate(record);
            let updated = record.clone();
            save_settings(&settings_path, &settings)?;
            return Ok(updated);
        }
    }
    anyhow::bail!("package {spec_or_id:?} is not installed")
}

/// Root a record's resources load from: the materialized store, or the
/// source path for local packages.
pub(crate) fn package_root(record: &PackageRecord) -> Option<PathBuf> {
    if let Some(install_path) = &record.install_path {
        return Some(PathBuf::from(install_path));
    }
    match &record.source {
        PackageSource::LocalPath { path } => Some(PathBuf::from(path)),
        _ => None,
    }
}

/// Parses a spec string into a source, resolving local paths to absolute
/// form (against `resolve_base` or the current directory) so identities are
/// stable.
pub(crate) fn resolve_spec_source(
    spec: &str,
    resolve_base: Option<&Path>,
) -> anyhow::Result<PackageSource> {
    let source = parse_package_spec(spec)?;
    match source {
        PackageSource::LocalPath { path } => Ok(PackageSource::LocalPath {
            path: resolve_local_path(&path, resolve_base)?,
        }),
        other => Ok(other),
    }
}

fn resolve_local_path(path: &str, resolve_base: Option<&Path>) -> anyhow::Result<String> {
    let expanded = if path == "~" {
        dirs::home_dir().context("cannot resolve `~`: no home directory")?
    } else if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .context("cannot resolve `~`: no home directory")?
            .join(rest)
    } else {
        PathBuf::from(path)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        let base = match resolve_base {
            Some(base) => base.to_path_buf(),
            None => std::env::current_dir().context("resolve relative package path")?,
        };
        base.join(expanded)
    };
    let normalized = fs::canonicalize(&absolute).unwrap_or(absolute);
    Ok(normalized.display().to_string())
}

/// Identity of a query string when it parses as a spec; used to match
/// records by identity in addition to package id and exact spec.
pub(crate) fn parsed_query_identity(query: &str) -> Option<PackageIdentity> {
    resolve_spec_source(query, None)
        .ok()
        .map(|source| source.identity())
}

/// npm invocation: explicit override, then `[packages] npm_command` from
/// config.toml, then plain `npm`.
pub(crate) fn resolve_npm_command(explicit: Option<Vec<String>>) -> Vec<String> {
    if let Some(command) = explicit.filter(|command| !command.is_empty()) {
        return command;
    }
    if let Ok(config) = crate::load_config()
        && let Some(packages) = config.packages
        && let Some(command) = packages.npm_command.filter(|command| !command.is_empty())
    {
        return command;
    }
    vec![DEFAULT_NPM_COMMAND.to_string()]
}
