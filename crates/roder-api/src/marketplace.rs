use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MarketplaceKind {
    Claude,
    Cursor,
    Codex,
    Roder,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DefaultMarketplaceSelection {
    None,
    Anthropic,
    Cursor,
    Codex,
    All,
}

impl DefaultMarketplaceSelection {
    pub fn selected_ids(&self) -> &'static [&'static str] {
        match self {
            Self::None => &[],
            Self::Anthropic => &["claude-plugins-official"],
            Self::Cursor => &["cursor-plugins"],
            Self::Codex => &["codex-plugins"],
            Self::All => &["claude-plugins-official", "cursor-plugins", "codex-plugins"],
        }
    }
}

impl std::str::FromStr for DefaultMarketplaceSelection {
    type Err = MarketplaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            "all" => Ok(Self::All),
            other => Err(MarketplaceError::InvalidDefaultSelection {
                selection: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MarketplaceSource {
    Github {
        repo: String,
        #[serde(rename = "refName")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(rename = "catalogPath")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        catalog_path: Option<String>,
        #[serde(rename = "pluginRoot")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plugin_root: Option<String>,
    },
    Git {
        url: String,
        #[serde(rename = "refName")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(rename = "catalogPath")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        catalog_path: Option<String>,
    },
    HttpJson {
        url: String,
    },
    LocalPath {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MarketplaceState {
    BakedIn,
    Installed,
    Refreshed,
    Disabled,
    RemovedByUser,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceDescriptor {
    pub id: String,
    pub kind: MarketplaceKind,
    pub display_name: String,
    pub source: MarketplaceSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_marketplace_state")]
    pub state: MarketplaceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_refreshed_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_marketplace_state() -> MarketplaceState {
    MarketplaceState::BakedIn
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PluginSource {
    MarketplacePath {
        marketplace_id: String,
        path: String,
    },
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(rename = "refName")]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha: Option<String>,
    },
    Npm {
        package: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    LocalPath {
        path: String,
    },
    Unsupported {
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginComponentHints {
    #[serde(default)]
    pub skills: bool,
    #[serde(default)]
    pub commands: bool,
    #[serde(default)]
    pub agents: bool,
    #[serde(default)]
    pub mcp_servers: bool,
    #[serde(default)]
    pub hooks: bool,
    #[serde(default)]
    pub apps: bool,
    #[serde(default)]
    pub lsp_servers: bool,
    #[serde(default)]
    pub rules: bool,
    #[serde(default)]
    pub assets: bool,
}

impl PluginComponentHints {
    pub fn command_capable(&self) -> bool {
        self.mcp_servers || self.hooks || self.apps || self.lsp_servers
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum MarketplacePluginRisk {
    Passive,
    ReadsWorkspace,
    StartsProcess,
    RunsHook,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PluginIdentityKey {
    pub canonical_slug: String,
    pub normalized_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePluginEntry {
    pub marketplace_id: String,
    pub plugin_id: String,
    pub identity_key: PluginIdentityKey,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: MarketplaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source: PluginSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub component_hints: PluginComponentHints,
    #[serde(default)]
    pub capability_hints: Vec<String>,
    pub risk: MarketplacePluginRisk,
    #[serde(default)]
    pub raw_manifest: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePluginVariant {
    pub marketplace_id: String,
    pub plugin_id: String,
    pub kind: MarketplaceKind,
    pub source: PluginSource,
    #[serde(default)]
    pub component_hints: PluginComponentHints,
    #[serde(default)]
    pub capability_hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub risk: MarketplacePluginRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DedupedMarketplacePlugin {
    pub identity_key: PluginIdentityKey,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub variants: Vec<MarketplacePluginVariant>,
    #[serde(default)]
    pub related_candidates: Vec<MarketplacePluginVariant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_variant_key: Option<String>,
    #[serde(default)]
    pub installed_variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MarketplaceInstallState {
    Previewed,
    Installed,
    Disabled,
    Uninstalled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginRecord {
    pub marketplace_id: String,
    pub plugin_id: String,
    pub identity_key: PluginIdentityKey,
    pub variant_key: String,
    pub install_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub state: MarketplaceInstallState,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MarketplaceError {
    InvalidMarketplaceId {
        id: String,
    },
    InvalidPluginId {
        id: String,
    },
    InvalidIdentityKey {
        key: String,
    },
    DuplicateMarketplace {
        id: String,
    },
    DuplicatePlugin {
        marketplace_id: String,
        plugin_id: String,
    },
    InvalidSource {
        message: String,
    },
    UnsupportedSource {
        message: String,
    },
    InvalidDefaultSelection {
        selection: String,
    },
    Io {
        message: String,
    },
    Parse {
        message: String,
    },
    NotFound {
        message: String,
    },
}

impl fmt::Display for MarketplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMarketplaceId { id } => write!(f, "invalid marketplace id `{id}`"),
            Self::InvalidPluginId { id } => write!(f, "invalid plugin id `{id}`"),
            Self::InvalidIdentityKey { key } => write!(f, "invalid plugin identity key `{key}`"),
            Self::DuplicateMarketplace { id } => write!(f, "duplicate marketplace `{id}`"),
            Self::DuplicatePlugin {
                marketplace_id,
                plugin_id,
            } => write!(
                f,
                "duplicate plugin `{plugin_id}` in marketplace `{marketplace_id}`"
            ),
            Self::InvalidSource { message } => write!(f, "invalid marketplace source: {message}"),
            Self::UnsupportedSource { message } => {
                write!(f, "unsupported marketplace source: {message}")
            }
            Self::InvalidDefaultSelection { selection } => {
                write!(f, "invalid default marketplace selection `{selection}`")
            }
            Self::Io { message } => write!(f, "marketplace io error: {message}"),
            Self::Parse { message } => write!(f, "marketplace parse error: {message}"),
            Self::NotFound { message } => write!(f, "marketplace entry not found: {message}"),
        }
    }
}

impl Error for MarketplaceError {}

pub fn validate_marketplace_id(id: &str) -> Result<(), MarketplaceError> {
    validate_slug(id).map_err(|_| MarketplaceError::InvalidMarketplaceId { id: id.to_string() })
}

pub fn validate_plugin_id(id: &str) -> Result<(), MarketplaceError> {
    validate_slug(id).map_err(|_| MarketplaceError::InvalidPluginId { id: id.to_string() })
}

pub fn validate_identity_key(identity: &PluginIdentityKey) -> Result<(), MarketplaceError> {
    if identity.canonical_slug.trim().is_empty()
        || identity.normalized_name.trim().is_empty()
        || normalize_slug(&identity.canonical_slug) != identity.canonical_slug
        || normalize_slug(&identity.normalized_name).is_empty()
    {
        return Err(MarketplaceError::InvalidIdentityKey {
            key: identity.canonical_slug.clone(),
        });
    }
    for value in [
        identity.repository.as_deref(),
        identity.homepage_domain.as_deref(),
        identity.author_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.trim().is_empty() {
            return Err(MarketplaceError::InvalidIdentityKey {
                key: identity.canonical_slug.clone(),
            });
        }
    }
    Ok(())
}

pub fn validate_marketplace_source(source: &MarketplaceSource) -> Result<(), MarketplaceError> {
    match source {
        MarketplaceSource::Github {
            repo,
            ref_name,
            catalog_path,
            plugin_root,
        } => {
            if repo.trim().is_empty()
                || repo.starts_with('/')
                || repo.contains("..")
                || repo.split('/').count() != 2
            {
                return invalid_source("github repo must be owner/repo");
            }
            validate_optional_path(catalog_path.as_deref(), "catalogPath")?;
            validate_optional_path(plugin_root.as_deref(), "pluginRoot")?;
            validate_optional_ref(ref_name.as_deref())?;
        }
        MarketplaceSource::Git {
            url,
            ref_name,
            catalog_path,
        } => {
            validate_url(url, &["https://", "ssh://", "git@", "file://"])?;
            validate_optional_path(catalog_path.as_deref(), "catalogPath")?;
            validate_optional_ref(ref_name.as_deref())?;
        }
        MarketplaceSource::HttpJson { url } => {
            validate_url(url, &["https://", "http://", "file://"])?;
        }
        MarketplaceSource::LocalPath { path } => validate_path_text(path, "local path")?,
    }
    Ok(())
}

pub fn validate_plugin_source(source: &PluginSource) -> Result<(), MarketplaceError> {
    match source {
        PluginSource::MarketplacePath {
            marketplace_id,
            path,
        } => {
            validate_marketplace_id(marketplace_id)?;
            validate_path_text(path, "marketplace path")?;
        }
        PluginSource::Git {
            url,
            path,
            ref_name,
            sha,
        } => {
            validate_url(url, &["https://", "ssh://", "git@", "file://"])?;
            validate_optional_path(path.as_deref(), "path")?;
            validate_optional_ref(ref_name.as_deref())?;
            validate_optional_ref(sha.as_deref())?;
        }
        PluginSource::Http { url, sha } => {
            validate_url(url, &["https://", "http://", "file://"])?;
            validate_optional_ref(sha.as_deref())?;
        }
        PluginSource::Npm { package, version } => {
            if package.trim().is_empty() || package.contains(char::is_whitespace) {
                return invalid_source("npm package must be non-empty and whitespace-free");
            }
            validate_optional_ref(version.as_deref())?;
        }
        PluginSource::LocalPath { path } => validate_path_text(path, "local path")?,
        PluginSource::Unsupported { value } => {
            return Err(MarketplaceError::UnsupportedSource {
                message: value.to_string(),
            });
        }
    }
    Ok(())
}

pub fn validate_plugin_entry(entry: &MarketplacePluginEntry) -> Result<(), MarketplaceError> {
    validate_marketplace_id(&entry.marketplace_id)?;
    validate_plugin_id(&entry.plugin_id)?;
    validate_identity_key(&entry.identity_key)?;
    validate_plugin_source(&entry.source)?;
    if entry.display_name.trim().is_empty() {
        return Err(MarketplaceError::InvalidPluginId {
            id: entry.plugin_id.clone(),
        });
    }
    Ok(())
}

fn validate_slug(value: &str) -> Result<(), ()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(());
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(());
    }
    let mut last = first;
    for ch in chars {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '.') {
            return Err(());
        }
        last = ch;
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(());
    }
    Ok(())
}

fn validate_url(value: &str, allowed_prefixes: &[&str]) -> Result<(), MarketplaceError> {
    let value = value.trim();
    if value.is_empty() {
        return invalid_source("url must be non-empty");
    }
    if allowed_prefixes
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        Ok(())
    } else {
        invalid_source(format!("url has unsupported scheme: {value}"))
    }
}

fn validate_optional_path(value: Option<&str>, label: &str) -> Result<(), MarketplaceError> {
    if let Some(value) = value {
        validate_relative_path_text(value, label)?;
    }
    Ok(())
}

fn validate_path_text(value: &str, label: &str) -> Result<(), MarketplaceError> {
    if value.trim().is_empty() {
        return invalid_source(format!("{label} must be non-empty"));
    }
    if value.split('/').any(|part| part == "..") {
        return invalid_source(format!("{label} must not contain '..'"));
    }
    Ok(())
}

fn validate_relative_path_text(value: &str, label: &str) -> Result<(), MarketplaceError> {
    validate_path_text(value, label)?;
    if value.starts_with('/') {
        return invalid_source(format!("{label} must be relative"));
    }
    Ok(())
}

fn validate_optional_ref(value: Option<&str>) -> Result<(), MarketplaceError> {
    if let Some(value) = value
        && (value.trim().is_empty() || value.contains(char::is_whitespace))
    {
        return invalid_source("ref, version, and sha values must be whitespace-free");
    }
    Ok(())
}

fn invalid_source(message: impl Into<String>) -> Result<(), MarketplaceError> {
    Err(MarketplaceError::InvalidSource {
        message: message.into(),
    })
}

pub fn normalize_slug(value: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    for ch in value.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            previous_dash = false;
        } else if !previous_dash {
            out.push('-');
            previous_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn variant_key(marketplace_id: &str, plugin_id: &str) -> String {
    format!("{marketplace_id}:{plugin_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpdated {
    pub marketplace: MarketplaceDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePluginInstalled {
    pub plugin: InstalledPluginRecord,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_sources_round_trip_camel_case() {
        let source = MarketplaceSource::Github {
            repo: "openai/plugins".to_string(),
            ref_name: Some("main".to_string()),
            catalog_path: None,
            plugin_root: Some("plugins".to_string()),
        };
        let value = serde_json::to_value(&source).unwrap();
        assert_eq!(value["kind"], "github");
        assert_eq!(value["pluginRoot"], "plugins");
        let decoded: MarketplaceSource = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, source);
    }

    #[test]
    fn plugin_sources_round_trip_supported_shapes() {
        for source in [
            PluginSource::Git {
                url: "https://github.com/openai/plugins.git".to_string(),
                path: Some("plugins/superpowers".to_string()),
                ref_name: Some("main".to_string()),
                sha: Some("abc".to_string()),
            },
            PluginSource::Npm {
                package: "@scope/plugin".to_string(),
                version: Some("1.0.0".to_string()),
            },
            PluginSource::MarketplacePath {
                marketplace_id: "codex-plugins".to_string(),
                path: "plugins/demo".to_string(),
            },
            PluginSource::Http {
                url: "https://example.test/plugin.zip".to_string(),
                sha: None,
            },
        ] {
            let value = serde_json::to_value(&source).unwrap();
            let decoded: PluginSource = serde_json::from_value(value).unwrap();
            assert_eq!(decoded, source);
        }
    }

    #[test]
    fn default_marketplace_selection_parses_expected_values() {
        assert_eq!(
            "anthropic".parse::<DefaultMarketplaceSelection>().unwrap(),
            DefaultMarketplaceSelection::Anthropic
        );
        assert_eq!(
            "claude".parse::<DefaultMarketplaceSelection>().unwrap(),
            DefaultMarketplaceSelection::Anthropic
        );
        assert_eq!(
            "all".parse::<DefaultMarketplaceSelection>().unwrap(),
            DefaultMarketplaceSelection::All
        );
        assert!("bogus".parse::<DefaultMarketplaceSelection>().is_err());
    }

    #[test]
    fn marketplace_validation_rejects_unsafe_sources_and_ids() {
        assert!(validate_marketplace_id("cursor-local").is_ok());
        assert!(validate_marketplace_id("Cursor Local").is_err());
        assert!(
            validate_marketplace_source(&MarketplaceSource::Github {
                repo: "owner/plugins".to_string(),
                ref_name: Some("main".to_string()),
                catalog_path: Some(".cursor-plugin/marketplace.json".to_string()),
                plugin_root: None,
            })
            .is_ok()
        );
        assert!(
            validate_marketplace_source(&MarketplaceSource::Github {
                repo: "../plugins".to_string(),
                ref_name: None,
                catalog_path: None,
                plugin_root: None,
            })
            .is_err()
        );
        assert!(
            validate_marketplace_source(&MarketplaceSource::Git {
                url: "ftp://example.test/plugins.git".to_string(),
                ref_name: None,
                catalog_path: None,
            })
            .is_err()
        );
        assert!(
            validate_marketplace_source(&MarketplaceSource::Github {
                repo: "owner/plugins".to_string(),
                ref_name: None,
                catalog_path: Some("../marketplace.json".to_string()),
                plugin_root: None,
            })
            .is_err()
        );
    }

    #[test]
    fn plugin_validation_rejects_bad_identity_and_unsupported_source() {
        let mut entry = MarketplacePluginEntry {
            marketplace_id: "cursor-local".to_string(),
            plugin_id: "repo-tools".to_string(),
            identity_key: PluginIdentityKey {
                canonical_slug: "repo-tools".to_string(),
                normalized_name: "repo tools".to_string(),
                repository: Some("https://github.com/example/repo-tools".to_string()),
                homepage_domain: Some("github.com".to_string()),
                author_name: None,
            },
            display_name: "Repo Tools".to_string(),
            description: None,
            kind: MarketplaceKind::Cursor,
            version: None,
            source: PluginSource::MarketplacePath {
                marketplace_id: "cursor-local".to_string(),
                path: "repo-tools".to_string(),
            },
            homepage: None,
            repository: None,
            author_name: None,
            category: None,
            tags: Vec::new(),
            component_hints: PluginComponentHints::default(),
            capability_hints: Vec::new(),
            risk: MarketplacePluginRisk::Passive,
            raw_manifest: serde_json::json!({ "name": "repo-tools" }),
        };
        assert!(validate_plugin_entry(&entry).is_ok());

        entry.identity_key.canonical_slug = "Repo Tools".to_string();
        assert!(validate_plugin_entry(&entry).is_err());

        entry.identity_key.canonical_slug = "repo-tools".to_string();
        entry.source = PluginSource::Unsupported {
            value: serde_json::json!({ "source": "unknown" }),
        };
        assert!(validate_plugin_entry(&entry).is_err());
    }

    #[test]
    fn marketplace_contract_structs_use_camel_case_fields() {
        let record = InstalledPluginRecord {
            marketplace_id: "codex-plugins".to_string(),
            plugin_id: "superpowers".to_string(),
            identity_key: PluginIdentityKey {
                canonical_slug: "superpowers".to_string(),
                normalized_name: "superpowers".to_string(),
                repository: Some("https://github.com/obra/superpowers".to_string()),
                homepage_domain: Some("github.com".to_string()),
                author_name: Some("Jesse Vincent".to_string()),
            },
            variant_key: variant_key("codex-plugins", "superpowers"),
            install_path: "/tmp/cache/superpowers".to_string(),
            version: Some("5.1.0".to_string()),
            content_hash: Some("hash".to_string()),
            state: MarketplaceInstallState::Installed,
            installed_at: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(record).unwrap();

        assert_eq!(value["marketplaceId"], "codex-plugins");
        assert_eq!(value["identityKey"]["canonicalSlug"], "superpowers");
        assert_eq!(value["installedAt"], "1970-01-01T00:00:00Z");
    }

    #[test]
    fn validates_slug_ids() {
        assert!(validate_marketplace_id("codex-plugins").is_ok());
        assert!(validate_plugin_id("superpowers.2").is_ok());
        assert!(validate_marketplace_id("Bad").is_err());
        assert!(validate_plugin_id("-bad").is_err());
    }
}
