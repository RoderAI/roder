use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use roder_api::distribution::{DistributionManifest, ExtensionCategory, Profile};
use serde::Deserialize;

use crate::catalog::Catalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltInProfile {
    pub id: &'static str,
    pub source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub required_env: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    Load {
        path: String,
        message: String,
    },
    UnknownExtension {
        id: String,
    },
    Conflict {
        first_id: String,
        second_id: String,
    },
    CapabilityDisabled {
        extension_id: String,
        capability: String,
    },
    CategoryCardinality {
        category: ExtensionCategory,
        count: usize,
    },
    MissingDefault {
        field: &'static str,
        id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    id: String,
    description: String,
    #[serde(rename = "distribution")]
    manifest: StrictDistributionManifest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictDistributionManifest {
    name: String,
    version: String,
    include_tui: bool,
    include_app_server: bool,
    include_cli: bool,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default)]
    default_thread_store: Option<String>,
    #[serde(default)]
    config_overrides: serde_json::Value,
}

pub trait ProfileExt {
    fn load(path: impl AsRef<Path>) -> Result<Profile, ProfileError>;
    fn from_toml(source: &str) -> Result<Profile, ProfileError>;
    fn validate(&self, catalog: &Catalog) -> Result<ValidationReport, ProfileError>;
}

impl ProfileExt for Profile {
    fn load(path: impl AsRef<Path>) -> Result<Profile, ProfileError> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path).map_err(|err| ProfileError::Load {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        parse_profile(&source, path.display().to_string())
    }

    fn from_toml(source: &str) -> Result<Profile, ProfileError> {
        parse_profile(source, "<inline>".to_string())
    }

    fn validate(&self, catalog: &Catalog) -> Result<ValidationReport, ProfileError> {
        validate_profile(self, catalog)
    }
}

pub fn built_in_profiles() -> Result<Vec<Profile>, ProfileError> {
    BUILT_IN_PROFILES
        .iter()
        .map(|profile| Profile::from_toml(profile.source))
        .collect()
}

pub fn built_in_profile(id: &str) -> Result<Option<Profile>, ProfileError> {
    BUILT_IN_PROFILES
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| Profile::from_toml(profile.source))
        .transpose()
}

pub const BUILT_IN_PROFILES: &[BuiltInProfile] = &[
    BuiltInProfile {
        id: "minimal",
        source: include_str!("../profiles/minimal.toml"),
    },
    BuiltInProfile {
        id: "openai-only",
        source: include_str!("../profiles/openai-only.toml"),
    },
    BuiltInProfile {
        id: "anthropic-only",
        source: include_str!("../profiles/anthropic-only.toml"),
    },
    BuiltInProfile {
        id: "research-headless",
        source: include_str!("../profiles/research-headless.toml"),
    },
    BuiltInProfile {
        id: "remote-app-server",
        source: include_str!("../profiles/remote-app-server.toml"),
    },
    BuiltInProfile {
        id: "tavily",
        source: include_str!("../profiles/tavily.toml"),
    },
    BuiltInProfile {
        id: "zero-coder-edits",
        source: include_str!("../profiles/zero-coder-edits.toml"),
    },
    BuiltInProfile {
        id: "full",
        source: include_str!("../profiles/full.toml"),
    },
];

fn parse_profile(source: &str, path: String) -> Result<Profile, ProfileError> {
    let parsed = toml::from_str::<ProfileFile>(source).map_err(|err| ProfileError::Load {
        path,
        message: err.to_string(),
    })?;
    Ok(Profile {
        id: parsed.id,
        description: parsed.description,
        manifest: DistributionManifest {
            name: parsed.manifest.name,
            version: parsed.manifest.version,
            include_tui: parsed.manifest.include_tui,
            include_app_server: parsed.manifest.include_app_server,
            include_cli: parsed.manifest.include_cli,
            extensions: parsed.manifest.extensions,
            default_provider: parsed.manifest.default_provider,
            default_thread_store: parsed.manifest.default_thread_store,
            config_overrides: parsed.manifest.config_overrides,
        },
    })
}

fn validate_profile(
    profile: &Profile,
    catalog: &Catalog,
) -> Result<ValidationReport, ProfileError> {
    let selected = profile
        .manifest
        .extensions
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let disabled_capabilities = disabled_capabilities(profile);
    let mut category_counts: BTreeMap<ExtensionCategory, usize> = BTreeMap::new();
    let mut required_env = BTreeMap::new();

    for id in &selected {
        let entry = catalog
            .get(id)
            .ok_or_else(|| ProfileError::UnknownExtension { id: id.clone() })?;
        *category_counts
            .entry(entry.entry.category.clone())
            .or_insert(0) += 1;
        if !entry.entry.required_env.is_empty() {
            required_env.insert(id.clone(), entry.entry.required_env.clone());
        }
        for capability in &entry.entry.required_capabilities {
            if disabled_capabilities.contains(capability) {
                return Err(ProfileError::CapabilityDisabled {
                    extension_id: id.clone(),
                    capability: capability.clone(),
                });
            }
        }
        for conflict in &entry.entry.conflicts_with {
            if selected.contains(conflict) {
                return Err(ProfileError::Conflict {
                    first_id: id.clone(),
                    second_id: conflict.clone(),
                });
            }
        }
    }

    for category in [
        ExtensionCategory::ThreadStore,
        ExtensionCategory::CheckpointStore,
        ExtensionCategory::MemoryStore,
    ] {
        if let Some(count) = category_counts.get(&category)
            && *count > 1
        {
            return Err(ProfileError::CategoryCardinality {
                category,
                count: *count,
            });
        }
    }

    if let Some(default_provider) = &profile.manifest.default_provider {
        require_selected(&selected, "default_provider", default_provider)?;
    }
    if let Some(default_thread_store) = &profile.manifest.default_thread_store {
        require_selected(&selected, "default_thread_store", default_thread_store)?;
    }

    Ok(ValidationReport { required_env })
}

fn disabled_capabilities(profile: &Profile) -> BTreeSet<String> {
    profile
        .manifest
        .config_overrides
        .get("disabled_capabilities")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect()
}

fn require_selected(
    selected: &BTreeSet<String>,
    field: &'static str,
    id: &str,
) -> Result<(), ProfileError> {
    if selected.contains(id) {
        Ok(())
    } else {
        Err(ProfileError::MissingDefault {
            field,
            id: id.to_string(),
        })
    }
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load { path, message } => write!(f, "failed to load profile {path}: {message}"),
            Self::UnknownExtension { id } => {
                write!(f, "profile references unknown extension `{id}`")
            }
            Self::Conflict {
                first_id,
                second_id,
            } => {
                write!(
                    f,
                    "profile selects conflicting extensions `{first_id}` and `{second_id}`"
                )
            }
            Self::CapabilityDisabled {
                extension_id,
                capability,
            } => write!(
                f,
                "profile selects `{extension_id}` but disables required capability `{capability}`"
            ),
            Self::CategoryCardinality { category, count } => {
                write!(
                    f,
                    "profile selects {count} entries for single-select category {category:?}"
                )
            }
            Self::MissingDefault { field, id } => {
                write!(
                    f,
                    "profile {field} `{id}` is not included in distribution.extensions"
                )
            }
        }
    }
}

impl std::error::Error for ProfileError {}
