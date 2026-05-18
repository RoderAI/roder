pub mod claude;
pub mod codex;
pub mod cursor;
pub mod dedupe;
pub mod install;

use roder_api::marketplace::{
    MarketplaceKind, MarketplacePluginEntry, MarketplacePluginRisk, PluginComponentHints,
    PluginIdentityKey, PluginSource, normalize_slug,
};
use roder_config::marketplaces::RawMarketplaceCatalog;

pub use dedupe::dedupe_plugins;
pub use install::{install_plugin_variant, preview_plugin_install};

pub fn normalize_catalog(
    catalog: &RawMarketplaceCatalog,
) -> anyhow::Result<Vec<MarketplacePluginEntry>> {
    match catalog.marketplace.kind {
        MarketplaceKind::Claude => claude::normalize_catalog(catalog),
        MarketplaceKind::Cursor => cursor::normalize_catalog(catalog),
        MarketplaceKind::Codex => codex::normalize_catalog(catalog),
        MarketplaceKind::Roder | MarketplaceKind::Custom => Ok(Vec::new()),
    }
}

pub(crate) fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn array_strings(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn author_name(value: &serde_json::Value) -> Option<String> {
    value
        .get("author")
        .and_then(|author| {
            author
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| string_field(author, "name"))
        })
        .or_else(|| string_field(value, "developerName"))
}

pub(crate) fn source_from_value(
    marketplace_id: &str,
    source: Option<&serde_json::Value>,
    default_path: Option<String>,
) -> PluginSource {
    match source {
        Some(serde_json::Value::String(path)) => PluginSource::MarketplacePath {
            marketplace_id: marketplace_id.to_string(),
            path: path.clone(),
        },
        Some(serde_json::Value::Object(map)) => {
            let kind = map
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            match kind {
                "git-subdir" => PluginSource::Git {
                    url: map
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    path: map
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                    ref_name: map
                        .get("ref")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                    sha: map
                        .get("sha")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                },
                "url" | "git" => PluginSource::Git {
                    url: map
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    path: None,
                    ref_name: map
                        .get("ref")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                    sha: map
                        .get("sha")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                },
                "npm" => PluginSource::Npm {
                    package: map
                        .get("package")
                        .or_else(|| map.get("name"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    version: map
                        .get("version")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                },
                _ => PluginSource::Unsupported {
                    value: serde_json::Value::Object(map.clone()),
                },
            }
        }
        _ => PluginSource::MarketplacePath {
            marketplace_id: marketplace_id.to_string(),
            path: default_path.unwrap_or_else(|| ".".to_string()),
        },
    }
}

pub(crate) fn component_hints(value: &serde_json::Value) -> PluginComponentHints {
    let mut hints = PluginComponentHints::default();
    hints.skills = value.get("skills").is_some();
    hints.commands = value.get("commands").is_some();
    hints.agents = value.get("agents").is_some();
    hints.mcp_servers = value.get("mcpServers").is_some() || value.get("mcp").is_some();
    hints.hooks = value.get("hooks").is_some() || value.get("hooksJson").is_some();
    hints.apps = value.get("app").is_some() || value.get("appJson").is_some();
    hints.lsp_servers = value.get("lspServers").is_some();
    hints.rules = value.get("rules").is_some();
    hints.assets = value.get("assets").is_some()
        || value
            .get("interface")
            .and_then(|interface| {
                interface
                    .get("logo")
                    .or_else(|| interface.get("composerIcon"))
                    .or_else(|| interface.get("screenshots"))
            })
            .is_some();
    hints
}

pub(crate) fn risk_for(
    source: &PluginSource,
    hints: &PluginComponentHints,
) -> MarketplacePluginRisk {
    if hints.hooks {
        MarketplacePluginRisk::RunsHook
    } else if hints.command_capable()
        || matches!(
            source,
            PluginSource::Npm { .. } | PluginSource::Unsupported { .. }
        )
    {
        MarketplacePluginRisk::StartsProcess
    } else {
        MarketplacePluginRisk::Passive
    }
}

pub(crate) fn identity_key(
    name: &str,
    repository: Option<String>,
    homepage: Option<String>,
    author_name: Option<String>,
    source: &PluginSource,
) -> PluginIdentityKey {
    let normalized_name = normalize_slug(name);
    let repository = repository.or_else(|| repository_from_source(source));
    let homepage_domain = homepage.as_deref().and_then(homepage_domain);
    let canonical_slug = repository
        .as_deref()
        .map(normalize_repo_url)
        .filter(|slug| !slug.is_empty())
        .unwrap_or_else(|| {
            if let Some(domain) = &homepage_domain {
                format!("{domain}:{normalized_name}")
            } else if let Some(author) = &author_name {
                format!("{}:{normalized_name}", normalize_slug(author))
            } else {
                normalized_name.clone()
            }
        });
    PluginIdentityKey {
        canonical_slug,
        normalized_name,
        repository,
        homepage_domain,
        author_name,
    }
}

fn repository_from_source(source: &PluginSource) -> Option<String> {
    match source {
        PluginSource::Git { url, .. } => Some(url.clone()),
        PluginSource::Http { url, .. } => Some(url.clone()),
        _ => None,
    }
}

fn homepage_domain(homepage: &str) -> Option<String> {
    let stripped = homepage
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped
        .split('/')
        .next()
        .filter(|domain| !domain.is_empty())
        .map(|domain| domain.trim_start_matches("www.").to_ascii_lowercase())
}

fn normalize_repo_url(value: &str) -> String {
    let value = value
        .trim()
        .trim_end_matches(".git")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("git@")
        .replace(':', "/");
    normalize_slug(&value)
}
