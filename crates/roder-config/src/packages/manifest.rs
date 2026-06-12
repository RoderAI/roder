//! Package manifest discovery at a package root.
//!
//! Precedence:
//! 1. `roder.toml` (canonical, works in plain git repos)
//! 2. `package.json` `"roder"` key
//! 3. Conventional directories (`extensions/`, `skills/`, `commands/`,
//!    `themes/`)
//!
//! A root with no manifest and no conventional directories still loads (it
//! is installable) but declares zero resources, with a diagnostic saying so.

use std::fs;
use std::path::Path;

use anyhow::Context;
use roder_api::packages::{
    PACKAGE_MANIFEST_FILE, PackageManifestSpec, PackageSource, derive_package_id,
    validate_package_id,
};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestSourceKind {
    RoderToml,
    PackageJson,
    Conventional,
}

#[derive(Debug, Clone)]
pub struct LoadedPackageManifest {
    pub spec: PackageManifestSpec,
    pub source: ManifestSourceKind,
}

#[derive(Debug, Deserialize)]
struct RoderTomlManifest {
    package: RoderTomlPackage,
    #[serde(default)]
    resources: Option<RoderTomlResources>,
}

#[derive(Debug, Deserialize)]
struct RoderTomlPackage {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RoderTomlResources {
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
}

/// The `"roder"` key inside `package.json`. `Option` fields distinguish
/// "absent" (fall back to conventional directories when all are absent)
/// from an explicit empty list.
#[derive(Debug, Default, Deserialize)]
struct PackageJsonRoderKey {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    skills: Option<Vec<String>>,
    #[serde(default)]
    commands: Option<Vec<String>>,
    #[serde(default)]
    themes: Option<Vec<String>>,
}

/// Loads the manifest for a package root. Hard errors (broken `roder.toml`,
/// invalid declared id) fail the load; soft problems are diagnostics.
pub fn load_package_manifest(
    root: &Path,
    source: &PackageSource,
) -> anyhow::Result<(LoadedPackageManifest, Vec<String>)> {
    let mut diagnostics = Vec::new();

    let roder_toml = root.join(PACKAGE_MANIFEST_FILE);
    if roder_toml.is_file() {
        let spec = load_roder_toml(&roder_toml, root)?;
        return Ok((
            LoadedPackageManifest {
                spec,
                source: ManifestSourceKind::RoderToml,
            },
            diagnostics,
        ));
    }

    let package_json = root.join("package.json");
    if package_json.is_file() {
        match read_package_json(&package_json) {
            Ok(value) => {
                if value.get("roder").is_some() {
                    let spec = load_package_json_roder(&value, &package_json, root, source)?;
                    return Ok((
                        LoadedPackageManifest {
                            spec,
                            source: ManifestSourceKind::PackageJson,
                        },
                        diagnostics,
                    ));
                }
            }
            Err(err) => {
                diagnostics.push(format!("{err:#}; falling back to conventional directories"))
            }
        }
    }

    let (extensions, skills, commands, themes) = conventional_resources(root);
    if extensions.is_empty() && skills.is_empty() && commands.is_empty() && themes.is_empty() {
        diagnostics.push(format!(
            "package at {} declares no resources: no {PACKAGE_MANIFEST_FILE}, no package.json \
             `roder` key, and no conventional extensions/, skills/, commands/, or themes/ \
             directories",
            root.display()
        ));
    }
    let id = derive_package_id(source);
    validate_package_id(&id)
        .with_context(|| format!("derived package id for {}", root.display()))?;
    Ok((
        LoadedPackageManifest {
            spec: PackageManifestSpec {
                id,
                name: None,
                version: None,
                description: None,
                extensions,
                skills,
                commands,
                themes,
            },
            source: ManifestSourceKind::Conventional,
        },
        diagnostics,
    ))
}

fn load_roder_toml(path: &Path, root: &Path) -> anyhow::Result<PackageManifestSpec> {
    let text =
        fs::read_to_string(path).with_context(|| format!("read manifest {}", path.display()))?;
    let manifest: RoderTomlManifest =
        toml::from_str(&text).with_context(|| format!("parse manifest {}", path.display()))?;
    validate_package_id(&manifest.package.id)
        .with_context(|| format!("manifest {}", path.display()))?;
    let (extensions, skills, commands, themes) = match manifest.resources {
        Some(resources) => (
            resources.extensions,
            resources.skills,
            resources.commands,
            resources.themes,
        ),
        // `[resources]` omitted: discover conventional directories.
        None => conventional_resources(root),
    };
    Ok(PackageManifestSpec {
        id: manifest.package.id,
        name: manifest.package.name,
        version: manifest.package.version,
        description: manifest.package.description,
        extensions,
        skills,
        commands,
        themes,
    })
}

fn read_package_json(path: &Path) -> anyhow::Result<serde_json::Value> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn load_package_json_roder(
    value: &serde_json::Value,
    path: &Path,
    root: &Path,
    source: &PackageSource,
) -> anyhow::Result<PackageManifestSpec> {
    let roder: PackageJsonRoderKey =
        serde_json::from_value(value.get("roder").cloned().unwrap_or_default())
            .with_context(|| format!("parse `roder` key in {}", path.display()))?;
    let id = match roder.id {
        Some(id) => {
            validate_package_id(&id).with_context(|| format!("manifest {}", path.display()))?;
            id
        }
        None => {
            let id = derive_package_id(source);
            validate_package_id(&id)
                .with_context(|| format!("derived package id for {}", root.display()))?;
            id
        }
    };
    let declared_any = roder.extensions.is_some()
        || roder.skills.is_some()
        || roder.commands.is_some()
        || roder.themes.is_some();
    let (extensions, skills, commands, themes) = if declared_any {
        (
            roder.extensions.unwrap_or_default(),
            roder.skills.unwrap_or_default(),
            roder.commands.unwrap_or_default(),
            roder.themes.unwrap_or_default(),
        )
    } else {
        // `"roder": { "id": ... }` without resource arrays: conventional.
        conventional_resources(root)
    };
    Ok(PackageManifestSpec {
        id,
        name: value
            .get("name")
            .and_then(|name| name.as_str())
            .map(str::to_string),
        version: value
            .get("version")
            .and_then(|version| version.as_str())
            .map(str::to_string),
        description: value
            .get("description")
            .and_then(|description| description.as_str())
            .map(str::to_string),
        extensions,
        skills,
        commands,
        themes,
    })
}

/// Conventional resource declarations for directories that exist at the
/// package root.
fn conventional_resources(root: &Path) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut extensions = Vec::new();
    if root.join("extensions").is_dir() {
        extensions.push("extensions/*/roder-extension.toml".to_string());
        extensions.push("extensions/*.toml".to_string());
    }
    let dir_entry = |name: &str| -> Vec<String> {
        if root.join(name).is_dir() {
            vec![name.to_string()]
        } else {
            Vec::new()
        }
    };
    (
        extensions,
        dir_entry("skills"),
        dir_entry("commands"),
        dir_entry("themes"),
    )
}
