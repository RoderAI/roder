use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use roder_api::distribution::{CatalogError, DistributionEntry};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    entries: BTreeMap<String, CatalogEntry>,
    extra_crates: Vec<ExtraCrate>,
    missing_metadata: Vec<CatalogError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub entry: DistributionEntry,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraCrate {
    pub name: String,
    pub source: ExtraCrateSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraCrateSource {
    Path(PathBuf),
    Git(String),
}

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
}

#[derive(Debug, Deserialize)]
struct MetadataPackage {
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    metadata: serde_json::Value,
}

impl Catalog {
    pub fn from_workspace(workspace: impl AsRef<Path>) -> anyhow::Result<Self> {
        let output = Command::new("cargo")
            .arg("metadata")
            .arg("--format-version=1")
            .arg("--no-deps")
            .current_dir(workspace)
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self::from_metadata_json(&output.stdout)
    }

    pub fn from_metadata_json(bytes: &[u8]) -> anyhow::Result<Self> {
        let metadata: Metadata = serde_json::from_slice(bytes)?;
        let mut entries: BTreeMap<String, CatalogEntry> = BTreeMap::new();
        let mut missing_metadata = Vec::new();

        for package in metadata.packages {
            let Some(distribution) = package
                .metadata
                .get("roder")
                .and_then(|roder| roder.get("distribution"))
                .cloned()
            else {
                missing_metadata.push(CatalogError::MissingMetadata {
                    crate_name: package.name,
                    manifest_path: Some(package.manifest_path.display().to_string()),
                });
                continue;
            };

            let entry =
                parse_distribution_entry(&package.name, &package.manifest_path, distribution)?;
            if let Some(existing) = entries.get(&entry.id) {
                return Err(CatalogError::Conflict {
                    first_id: existing.entry.id.clone(),
                    second_id: entry.id.clone(),
                    reason: format!(
                        "duplicate id declared by {} and {}",
                        existing.manifest_path.display(),
                        package.manifest_path.display()
                    ),
                }
                .into());
            }
            entries.insert(
                entry.id.clone(),
                CatalogEntry {
                    entry,
                    manifest_path: package.manifest_path,
                },
            );
        }

        Ok(Self {
            entries,
            extra_crates: Vec::new(),
            missing_metadata,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }

    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.get(id)
    }

    pub fn missing_metadata(&self) -> &[CatalogError] {
        &self.missing_metadata
    }

    pub fn extra_crates(&self) -> &[ExtraCrate] {
        &self.extra_crates
    }

    pub fn add_extra_crate(&mut self, spec: &str) -> Result<(), CatalogError> {
        self.extra_crates.push(parse_extra_crate(spec)?);
        Ok(())
    }
}

fn parse_distribution_entry(
    crate_name: &str,
    manifest_path: &Path,
    distribution: serde_json::Value,
) -> Result<DistributionEntry, CatalogError> {
    let mut object =
        distribution
            .as_object()
            .cloned()
            .ok_or_else(|| CatalogError::MalformedMetadata {
                crate_name: crate_name.to_string(),
                manifest_path: Some(manifest_path.display().to_string()),
                message: "expected table".to_string(),
            })?;
    object
        .entry("crate_name".to_string())
        .or_insert_with(|| serde_json::Value::String(crate_name.to_string()));

    serde_json::from_value(serde_json::Value::Object(object)).map_err(|err| {
        CatalogError::MalformedMetadata {
            crate_name: crate_name.to_string(),
            manifest_path: Some(manifest_path.display().to_string()),
            message: err.to_string(),
        }
    })
}

fn parse_extra_crate(spec: &str) -> Result<ExtraCrate, CatalogError> {
    let (name, source) = spec
        .split_once('=')
        .ok_or_else(|| malformed_extra_crate(spec, "expected <name>=<path|git>"))?;
    let name = name.trim();
    let source = source.trim();
    if name.is_empty() || source.is_empty() {
        return Err(malformed_extra_crate(
            spec,
            "name and source must be non-empty",
        ));
    }
    let source = if source.starts_with("git+") || source.starts_with("https://") {
        ExtraCrateSource::Git(source.trim_start_matches("git+").to_string())
    } else {
        ExtraCrateSource::Path(PathBuf::from(source))
    };
    Ok(ExtraCrate {
        name: name.to_string(),
        source,
    })
}

fn malformed_extra_crate(spec: &str, message: &str) -> CatalogError {
    CatalogError::MalformedMetadata {
        crate_name: spec.to_string(),
        manifest_path: None,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use roder_api::distribution::ExtensionCategory;

    use super::*;

    #[test]
    fn catalog_discovers_distribution_metadata() {
        let catalog = Catalog::from_metadata_json(
            metadata_json(vec![
                package(
                    "roder-ext-a",
                    "a",
                    Some(entry("a", "tool-provider", vec![])),
                ),
                package(
                    "roder-ext-b",
                    "b",
                    Some(entry("b", "inference-engine", vec![])),
                ),
                package("roder-ext-c", "c", Some(entry("c", "event-sink", vec![]))),
                package("roder-ext-no-meta", "missing", None),
            ])
            .to_string()
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(catalog.entries().count(), 3);
        assert_eq!(
            catalog.get("a").unwrap().entry.category,
            ExtensionCategory::ToolProvider
        );
        assert_eq!(catalog.missing_metadata().len(), 1);
    }

    #[test]
    fn catalog_rejects_duplicate_ids() {
        let err = Catalog::from_metadata_json(
            metadata_json(vec![
                package(
                    "roder-ext-a",
                    "a",
                    Some(entry("dup", "tool-provider", vec![])),
                ),
                package("roder-ext-b", "b", Some(entry("dup", "event-sink", vec![]))),
            ])
            .to_string()
            .as_bytes(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("duplicate id"));
    }

    #[test]
    fn catalog_preserves_declared_conflicts_for_profile_validation() {
        let catalog = Catalog::from_metadata_json(
            metadata_json(vec![
                package(
                    "roder-ext-a",
                    "a",
                    Some(entry("a", "tool-provider", vec!["b"])),
                ),
                package("roder-ext-b", "b", Some(entry("b", "event-sink", vec![]))),
            ])
            .to_string()
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(
            catalog.get("a").unwrap().entry.conflicts_with,
            vec!["b".to_string()]
        );
    }

    #[test]
    fn extra_crate_specs_capture_path_and_git_sources() {
        let mut catalog =
            Catalog::from_metadata_json(metadata_json(vec![]).to_string().as_bytes()).unwrap();

        catalog.add_extra_crate("local=../local-ext").unwrap();
        catalog
            .add_extra_crate("remote=git+https://example.com/ext.git")
            .unwrap();

        assert_eq!(
            catalog.extra_crates()[0],
            ExtraCrate {
                name: "local".to_string(),
                source: ExtraCrateSource::Path(PathBuf::from("../local-ext")),
            }
        );
        assert_eq!(
            catalog.extra_crates()[1],
            ExtraCrate {
                name: "remote".to_string(),
                source: ExtraCrateSource::Git("https://example.com/ext.git".to_string()),
            }
        );
    }

    fn metadata_json(packages: Vec<serde_json::Value>) -> serde_json::Value {
        json!({ "packages": packages })
    }

    fn package(
        name: &str,
        manifest_name: &str,
        distribution: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let metadata = distribution
            .map(|distribution| json!({ "roder": { "distribution": distribution } }))
            .unwrap_or_else(|| json!({}));
        json!({
            "name": name,
            "manifest_path": format!("/tmp/{manifest_name}/Cargo.toml"),
            "metadata": metadata,
        })
    }

    fn entry(id: &str, category: &str, conflicts_with: Vec<&str>) -> serde_json::Value {
        json!({
            "id": id,
            "category": category,
            "display_name": format!("Extension {id}"),
            "description": "fixture extension",
            "default_in_profiles": ["full"],
            "required_env": [],
            "optional_env": [],
            "conflicts_with": conflicts_with,
            "required_capabilities": [],
            "extension_path": "::extension",
        })
    }
}
