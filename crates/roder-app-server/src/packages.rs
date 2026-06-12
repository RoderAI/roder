//! `packages/*` JSON-RPC handlers (roadmap phase 97): install, list, and
//! manage Roder packages through the `roder_config::packages` ops layer.
//! All methods operate on the server's configured workspace.

use std::path::PathBuf;

use roder_api::packages::{PackageRecord, PackageScope, PackageSource, parse_package_resource_id};
use roder_config::packages::{
    InstallOptions, PackagePaths, SyncStatus, UpdateStatus, approve_extensions,
    enumerate_resources, install_package, list_packages, load_package_manifest, load_settings,
    remove_package, set_filters, set_package_enabled, set_resource_enabled, sync_project_packages,
    update_packages,
};
use roder_protocol::{
    JsonRpcError, PackageDescriptor, PackageResourceDescriptor, PackageSyncOutcome,
    PackageSyncStatus, PackageUpdateOutcome, PackageUpdateStatus, PackagesApproveExtensionsParams,
    PackagesApproveExtensionsResult, PackagesInstallParams, PackagesInstallResult,
    PackagesListResult, PackagesRemoveParams, PackagesRemoveResult, PackagesSetEnabledParams,
    PackagesSetEnabledResult, PackagesSetFiltersParams, PackagesSetFiltersResult,
    PackagesSyncResult, PackagesUpdateParams, PackagesUpdateResult,
};

use crate::server::AppServer;

impl AppServer {
    /// Standard package paths rooted at the server's workspace (falling back
    /// to the process working directory).
    async fn package_paths(&self) -> PackagePaths {
        let workspace = self
            .runtime
            .status()
            .await
            .workspace
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok());
        PackagePaths::standard(workspace.as_deref())
    }

    pub(crate) async fn handle_packages_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let listed = list_packages(&paths).map_err(internal_error)?;
        let mut diagnostics = Vec::new();
        let packages = listed
            .into_iter()
            .map(|entry| {
                describe_package(entry.record, entry.shadowed_by_project, &mut diagnostics)
            })
            .collect();
        json_result(PackagesListResult {
            packages,
            diagnostics,
        })
    }

    pub(crate) async fn handle_packages_install(
        &self,
        params: PackagesInstallParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let installed = install_package(
            &paths,
            params.scope,
            &params.spec,
            InstallOptions {
                allow_scripts: params.allow_scripts.unwrap_or(false),
                resolve_base: paths.workspace.clone(),
                ..InstallOptions::default()
            },
        )
        .map_err(internal_error)?;
        let shadowed = shadowed_by_project(&paths, &installed.record);
        json_result(PackagesInstallResult {
            package: PackageDescriptor {
                record: installed.record,
                shadowed_by_project: shadowed,
                resources: installed.resources.into_iter().map(Into::into).collect(),
            },
            diagnostics: installed.diagnostics,
        })
    }

    pub(crate) async fn handle_packages_remove(
        &self,
        params: PackagesRemoveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let scopes: Vec<PackageScope> = match params.scope {
            Some(scope) => vec![scope],
            // Project scope first, mirroring shadowing.
            None => [PackageScope::Project, PackageScope::User]
                .into_iter()
                .filter(|scope| *scope != PackageScope::Project || paths.workspace.is_some())
                .collect(),
        };
        let mut last_error = None;
        for scope in scopes {
            match remove_package(&paths, scope, &params.spec_or_id) {
                Ok(removed) => return json_result(PackagesRemoveResult { removed }),
                Err(err) => last_error = Some(err),
            }
        }
        Err(internal_error(
            last_error
                .map(|err| format!("{err:#}"))
                .unwrap_or_else(|| format!("package {:?} is not installed", params.spec_or_id)),
        ))
    }

    pub(crate) async fn handle_packages_update(
        &self,
        params: PackagesUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let outcomes = update_packages(&paths, None, params.target.as_deref())
            .map_err(internal_error)?
            .into_iter()
            .map(|outcome| {
                let (status, resolved, message) = match outcome.status {
                    UpdateStatus::Updated { resolved } => {
                        (PackageUpdateStatus::Updated, resolved, None)
                    }
                    UpdateStatus::SkippedPinned => (PackageUpdateStatus::SkippedPinned, None, None),
                    UpdateStatus::Failed { message } => {
                        (PackageUpdateStatus::Failed, None, Some(message))
                    }
                };
                PackageUpdateOutcome {
                    package_id: outcome.package_id,
                    identity: outcome.identity.to_string(),
                    scope: outcome.scope,
                    status,
                    resolved,
                    message,
                }
            })
            .collect();
        json_result(PackagesUpdateResult { outcomes })
    }

    pub(crate) async fn handle_packages_sync(&self) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let outcomes = sync_project_packages(&paths)
            .map_err(internal_error)?
            .into_iter()
            .map(|outcome| {
                let (status, resolved, message) = match outcome.status {
                    SyncStatus::Materialized { resolved } => {
                        (PackageSyncStatus::Materialized, resolved, None)
                    }
                    SyncStatus::AlreadyPresent => (PackageSyncStatus::AlreadyPresent, None, None),
                    SyncStatus::Failed { message } => {
                        (PackageSyncStatus::Failed, None, Some(message))
                    }
                };
                PackageSyncOutcome {
                    package_id: outcome.package_id,
                    identity: outcome.identity.to_string(),
                    status,
                    resolved,
                    message,
                }
            })
            .collect();
        json_result(PackagesSyncResult { outcomes })
    }

    pub(crate) async fn handle_packages_set_enabled(
        &self,
        params: PackagesSetEnabledParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let record = if parse_package_resource_id(&params.id).is_ok() {
            set_resource_enabled(&paths, &params.id, params.enabled)
        } else {
            set_package_enabled(&paths, &params.id, params.enabled)
        }
        .map_err(internal_error)?;
        json_result(PackagesSetEnabledResult {
            package: self.describe_updated(&paths, record),
        })
    }

    pub(crate) async fn handle_packages_approve_extensions(
        &self,
        params: PackagesApproveExtensionsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let record = approve_extensions(&paths, &params.package_id, params.approved)
            .map_err(internal_error)?;
        json_result(PackagesApproveExtensionsResult {
            package: self.describe_updated(&paths, record),
        })
    }

    pub(crate) async fn handle_packages_set_filters(
        &self,
        params: PackagesSetFiltersParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let paths = self.package_paths().await;
        let record =
            set_filters(&paths, &params.package_id, params.filters).map_err(internal_error)?;
        json_result(PackagesSetFiltersResult {
            package: self.describe_updated(&paths, record),
        })
    }

    fn describe_updated(&self, paths: &PackagePaths, record: PackageRecord) -> PackageDescriptor {
        let shadowed = shadowed_by_project(paths, &record);
        describe_package(record, shadowed, &mut Vec::new())
    }
}

/// Builds the protocol descriptor for a record, enumerating its resources.
/// Disabled packages still enumerate (their resources report `enabled:
/// false`); enumeration problems land in `diagnostics`.
fn describe_package(
    record: PackageRecord,
    shadowed_by_project: bool,
    diagnostics: &mut Vec<String>,
) -> PackageDescriptor {
    let resources = package_resources(&record, diagnostics);
    PackageDescriptor {
        record,
        shadowed_by_project,
        resources,
    }
}

fn package_resources(
    record: &PackageRecord,
    diagnostics: &mut Vec<String>,
) -> Vec<PackageResourceDescriptor> {
    let root = match (&record.install_path, &record.source) {
        (Some(install_path), _) => PathBuf::from(install_path),
        (None, PackageSource::LocalPath { path }) => PathBuf::from(path),
        (None, _) => {
            diagnostics.push(format!(
                "package {} has no materialized root; run packages/sync or reinstall",
                record.package_id
            ));
            return Vec::new();
        }
    };
    if !root.is_dir() {
        diagnostics.push(format!(
            "package {} root {} is missing; run packages/sync or reinstall",
            record.package_id,
            root.display()
        ));
        return Vec::new();
    }
    let manifest = match load_package_manifest(&root, &record.source) {
        Ok((manifest, manifest_diagnostics)) => {
            diagnostics.extend(manifest_diagnostics);
            manifest
        }
        Err(err) => {
            diagnostics.push(format!("{err:#}"));
            return Vec::new();
        }
    };
    let (resources, resource_diagnostics) = enumerate_resources(&root, &manifest.spec, record);
    diagnostics.extend(resource_diagnostics);
    resources.into_iter().map(Into::into).collect()
}

/// True for a user-scope record whose identity is also installed in the
/// project scope (the project entry wins for this workspace).
fn shadowed_by_project(paths: &PackagePaths, record: &PackageRecord) -> bool {
    if record.scope != PackageScope::User || paths.workspace.is_none() {
        return false;
    }
    let Ok(settings_path) = paths.settings_path(PackageScope::Project) else {
        return false;
    };
    let Ok(settings) = load_settings(&settings_path) else {
        return false;
    };
    settings
        .packages
        .iter()
        .any(|other| other.identity == record.identity)
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

fn json_result<T: serde::Serialize>(value: T) -> Result<serde_json::Value, JsonRpcError> {
    serde_json::to_value(value).map_err(internal_error)
}
