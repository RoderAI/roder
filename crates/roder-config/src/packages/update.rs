//! Update and project-sync operations: re-fetching installed packages and
//! materializing missing project stores.

use std::path::PathBuf;

use anyhow::Context;
use roder_api::packages::{PackageIdentity, PackageRecord, PackageScope, PackageSource};

use super::fsutil::content_hash;
use super::git::{git_fetch_into_store, git_reconcile_existing};
use super::npm::npm_fetch_into_store;
use super::ops::{package_root, parsed_query_identity, resolve_npm_command};
use super::paths::PackagePaths;
use super::settings::{load_settings, save_settings};

#[derive(Debug)]
pub struct UpdateOutcome {
    pub package_id: String,
    pub identity: PackageIdentity,
    pub scope: PackageScope,
    pub status: UpdateStatus,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Updated { resolved: Option<String> },
    SkippedPinned,
    Failed { message: String },
}

#[derive(Debug)]
pub struct SyncOutcome {
    pub package_id: String,
    pub identity: PackageIdentity,
    pub status: SyncStatus,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SyncStatus {
    Materialized { resolved: Option<String> },
    AlreadyPresent,
    Failed { message: String },
}

/// Updates installed packages.
///
/// With a `target`, that package updates regardless of pin (a pinned npm
/// version reinstalls exactly; a pinned git ref reconciles the clone to it).
/// Bulk update skips pinned npm packages but still reconciles pinned git
/// clones to their ref.
pub fn update_packages(
    paths: &PackagePaths,
    scope_filter: Option<PackageScope>,
    target: Option<&str>,
) -> anyhow::Result<Vec<UpdateOutcome>> {
    // Project scope first so a targeted update hits the winning record.
    let scopes: Vec<PackageScope> = [PackageScope::Project, PackageScope::User]
        .into_iter()
        .filter(|scope| scope_filter.is_none_or(|filter| filter == *scope))
        .filter(|scope| *scope != PackageScope::Project || paths.workspace.is_some())
        .collect();
    let npm_command = resolve_npm_command(None);
    let mut outcomes = Vec::new();

    if let Some(target) = target {
        let parsed_identity = parsed_query_identity(target);
        for scope in scopes {
            let settings_path = paths.settings_path(scope)?;
            let settings = load_settings(&settings_path)?;
            let Some(record) = settings.find(target, parsed_identity.as_ref()).cloned() else {
                continue;
            };
            outcomes.push(update_one(paths, scope, &record, &npm_command));
            return Ok(outcomes);
        }
        anyhow::bail!("package {target:?} is not installed");
    }

    for scope in scopes {
        let settings_path = paths.settings_path(scope)?;
        let settings = load_settings(&settings_path)?;
        for record in settings.packages.clone() {
            let skip_pinned_npm =
                matches!(record.source, PackageSource::Npm { .. }) && record.source.pinned();
            if skip_pinned_npm {
                outcomes.push(UpdateOutcome {
                    package_id: record.package_id.clone(),
                    identity: record.identity.clone(),
                    scope,
                    status: UpdateStatus::SkippedPinned,
                });
                continue;
            }
            outcomes.push(update_one(paths, scope, &record, &npm_command));
        }
    }
    Ok(outcomes)
}

fn update_one(
    paths: &PackagePaths,
    scope: PackageScope,
    record: &PackageRecord,
    npm_command: &[String],
) -> UpdateOutcome {
    let result = refresh_record(paths, scope, record, npm_command);
    UpdateOutcome {
        package_id: record.package_id.clone(),
        identity: record.identity.clone(),
        scope,
        status: match result {
            Ok(resolved) => UpdateStatus::Updated { resolved },
            Err(err) => UpdateStatus::Failed {
                message: format!("{err:#}"),
            },
        },
    }
}

/// Re-fetches one record's contents and persists refreshed `resolved` and
/// `content_hash`. Returns the new resolved version/commit.
fn refresh_record(
    paths: &PackagePaths,
    scope: PackageScope,
    record: &PackageRecord,
    npm_command: &[String],
) -> anyhow::Result<Option<String>> {
    let (root, resolved) = match &record.source {
        PackageSource::Npm { name, version } => {
            let store = paths
                .store_path(scope, &record.source)?
                .expect("npm sources always have a store path");
            let outcome = npm_fetch_into_store(
                npm_command,
                name,
                version.as_deref(),
                record.allow_scripts,
                &store,
            )?;
            (store, outcome.resolved_version)
        }
        PackageSource::Git { url, ref_name } => {
            let store = paths
                .store_path(scope, &record.source)?
                .expect("git sources always have a store path");
            let outcome = if store.join(".git").exists() {
                git_reconcile_existing(
                    &store,
                    ref_name.as_deref(),
                    npm_command,
                    record.allow_scripts,
                )?
            } else {
                git_fetch_into_store(
                    url,
                    ref_name.as_deref(),
                    &store,
                    npm_command,
                    record.allow_scripts,
                )?
            };
            (store, outcome.resolved_commit)
        }
        PackageSource::LocalPath { path } => {
            let root = PathBuf::from(path);
            anyhow::ensure!(
                root.is_dir(),
                "local package path {} no longer exists",
                root.display()
            );
            (root, record.resolved.clone())
        }
    };
    let hash = content_hash(&root)
        .with_context(|| format!("hash package contents at {}", root.display()))?;

    let settings_path = paths.settings_path(scope)?;
    let mut settings = load_settings(&settings_path)?;
    if let Some(stored) = settings
        .packages
        .iter_mut()
        .find(|stored| stored.identity == record.identity)
    {
        stored.resolved = resolved.clone();
        stored.content_hash = Some(hash);
        save_settings(&settings_path, &settings)?;
    }
    Ok(resolved)
}

/// Materializes stores for project records that are missing on disk (e.g. a
/// freshly cloned repo with a committed `.roder/packages.json`).
///
/// This is an explicit command by design: there is no workspace trust gate
/// yet, so nothing auto-installs on startup. Scripts never run here even if
/// the committed record granted `allow_scripts` — that grant belonged to the
/// machine that made it.
pub fn sync_project_packages(paths: &PackagePaths) -> anyhow::Result<Vec<SyncOutcome>> {
    let settings_path = paths.settings_path(PackageScope::Project)?;
    let settings = load_settings(&settings_path)?;
    let npm_command = resolve_npm_command(None);
    let mut outcomes = Vec::new();
    for record in settings.packages.clone() {
        let root = package_root(&record);
        if root.as_ref().is_some_and(|root| root.is_dir()) {
            outcomes.push(SyncOutcome {
                package_id: record.package_id.clone(),
                identity: record.identity.clone(),
                status: SyncStatus::AlreadyPresent,
            });
            continue;
        }
        let status = if matches!(record.source, PackageSource::LocalPath { .. }) {
            SyncStatus::Failed {
                message: format!(
                    "local package path {} is missing on this machine",
                    record.source.spec()
                ),
            }
        } else {
            let mut sanitized = record.clone();
            sanitized.allow_scripts = false;
            match refresh_record(paths, PackageScope::Project, &sanitized, &npm_command) {
                Ok(resolved) => SyncStatus::Materialized { resolved },
                Err(err) => SyncStatus::Failed {
                    message: format!("{err:#}"),
                },
            }
        };
        outcomes.push(SyncOutcome {
            package_id: record.package_id.clone(),
            identity: record.identity.clone(),
            status,
        });
    }
    Ok(outcomes)
}
