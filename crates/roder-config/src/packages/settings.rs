//! `packages.json` settings store, one file per scope
//! (`<user_dir>/packages.json` and `<workspace>/.roder/packages.json`).

use std::fs;
use std::path::Path;

use anyhow::Context;
use roder_api::packages::{PackageIdentity, PackageRecord};
use serde::{Deserialize, Serialize};

use super::fsutil::unique_suffix;

pub const PACKAGES_SETTINGS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagesSettings {
    pub version: u32,
    #[serde(default)]
    pub packages: Vec<PackageRecord>,
}

impl Default for PackagesSettings {
    fn default() -> Self {
        Self {
            version: PACKAGES_SETTINGS_VERSION,
            packages: Vec::new(),
        }
    }
}

impl PackagesSettings {
    /// Inserts or replaces the record with the same identity, keeping the
    /// list sorted so re-installs are idempotent and diffs stay stable.
    pub fn upsert(&mut self, record: PackageRecord) {
        self.packages
            .retain(|existing| existing.identity != record.identity);
        self.packages.push(record);
        self.packages
            .sort_by(|a, b| (&a.package_id, &a.identity).cmp(&(&b.package_id, &b.identity)));
    }

    pub fn remove(&mut self, identity: &PackageIdentity) -> Option<PackageRecord> {
        let index = self
            .packages
            .iter()
            .position(|record| &record.identity == identity)?;
        Some(self.packages.remove(index))
    }

    pub fn find(
        &self,
        query: &str,
        parsed_identity: Option<&PackageIdentity>,
    ) -> Option<&PackageRecord> {
        self.packages
            .iter()
            .find(|record| record_matches(record, query, parsed_identity))
    }

    pub fn find_mut(
        &mut self,
        query: &str,
        parsed_identity: Option<&PackageIdentity>,
    ) -> Option<&mut PackageRecord> {
        self.packages
            .iter_mut()
            .find(|record| record_matches(record, query, parsed_identity))
    }
}

/// Matches a record against a user-supplied spec string or package id.
/// `parsed_identity` is the identity of the query when it parses as a spec
/// (with local paths already resolved to absolute form).
pub fn record_matches(
    record: &PackageRecord,
    query: &str,
    parsed_identity: Option<&PackageIdentity>,
) -> bool {
    record.package_id == query
        || record.source.spec() == query
        || parsed_identity.is_some_and(|identity| &record.identity == identity)
}

/// Loads a scope's settings; a missing file is an empty settings set.
pub fn load_settings(path: &Path) -> anyhow::Result<PackagesSettings> {
    if !path.exists() {
        return Ok(PackagesSettings::default());
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("read package settings {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parse package settings {}", path.display()))
}

/// Atomic save: write a temp sibling, then rename over the settings file.
pub fn save_settings(path: &Path, settings: &PackagesSettings) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create settings directory {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    let temp = path.with_extension(format!("json.tmp-{}", unique_suffix()));
    fs::write(&temp, text).with_context(|| format!("write {}", temp.display()))?;
    if let Err(err) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(err).with_context(|| format!("replace package settings {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use roder_api::packages::{PackageScope, PackageSource, parse_package_spec};
    use time::OffsetDateTime;

    use super::*;

    fn record(spec: &str, package_id: &str) -> PackageRecord {
        let source = parse_package_spec(spec).unwrap();
        PackageRecord {
            package_id: package_id.to_string(),
            identity: source.identity(),
            source,
            scope: PackageScope::User,
            install_path: None,
            resolved: None,
            enabled: true,
            allow_scripts: false,
            extensions_approved: false,
            installed_at: OffsetDateTime::now_utc(),
            content_hash: None,
            filters: Default::default(),
            disabled_resources: Vec::new(),
        }
    }

    #[test]
    fn settings_round_trip_and_upsert_dedupe() {
        let dir = std::env::temp_dir().join(format!("roder-pkg-settings-{}", unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("packages.json");

        assert!(load_settings(&path).unwrap().packages.is_empty());

        let mut settings = PackagesSettings::default();
        settings.upsert(record("npm:demo", "demo"));
        settings.upsert(record("npm:demo@1.0.0", "demo"));
        assert_eq!(settings.packages.len(), 1);
        assert_eq!(
            settings.packages[0].source,
            PackageSource::Npm {
                name: "demo".to_string(),
                version: Some("1.0.0".to_string())
            }
        );

        save_settings(&path, &settings).unwrap();
        let loaded = load_settings(&path).unwrap();
        assert_eq!(loaded.version, PACKAGES_SETTINGS_VERSION);
        assert_eq!(loaded.packages.len(), 1);
        assert!(loaded.find("demo", None).is_some());
        assert!(loaded.find("npm:demo@1.0.0", None).is_some());
        let identity = parse_package_spec("npm:demo@2.0.0").unwrap().identity();
        assert!(loaded.find("anything", Some(&identity)).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
