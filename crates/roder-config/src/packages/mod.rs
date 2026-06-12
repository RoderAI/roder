//! Roder package fetch/store/settings layer (roadmap phase 97).
//!
//! One `roder install <spec>` materializes an npm package, git clone, or
//! local path into a per-scope store (`~/.roder/packages/` or
//! `<workspace>/.roder/packages/`), records it in the scope's
//! `packages.json`, and enumerates the bundled process extensions, skills,
//! slash commands, and themes against the canonical contracts in
//! `roder_api::packages`.
//!
//! Safety posture: npm lifecycle scripts never run unless `allow_scripts`
//! was granted, and process extensions never launch until a package's
//! `extensions_approved` flag is set by an explicit approval step. This
//! layer never executes package-provided code.

mod fsutil;
mod git;
mod manifest;
mod npm;
mod ops;
mod paths;
mod resources;
mod settings;
mod snapshot;
mod update;

use serde::{Deserialize, Serialize};

pub use fsutil::content_hash;
pub use manifest::{LoadedPackageManifest, ManifestSourceKind, load_package_manifest};
pub use ops::{
    InstallOptions, InstalledPackage, ListedPackage, approve_extensions, install_package,
    list_packages, remove_package, set_filters, set_package_enabled, set_resource_enabled,
};
pub use paths::{
    PackagePaths, RODER_EPHEMERAL_APPROVE_ENV, RODER_EPHEMERAL_PACKAGES_ENV, git_store_components,
    npm_store_dir_name,
};
pub use resources::enumerate_resources;
pub use settings::{PACKAGES_SETTINGS_VERSION, PackagesSettings, load_settings, save_settings};
pub use snapshot::{
    PackageSnapshot, enabled_package_resources, package_command_dirs, package_process_extensions,
    package_skill_roots, package_theme_dirs,
};
pub use update::{
    SyncOutcome, SyncStatus, UpdateOutcome, UpdateStatus, sync_project_packages, update_packages,
};

/// `[packages]` section of `config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackagesConfig {
    /// Wrapper for npm operations, e.g.
    /// `npm_command = ["mise", "exec", "node@20", "--", "npm"]`.
    /// Defaults to `["npm"]`.
    pub npm_command: Option<Vec<String>>,
}
