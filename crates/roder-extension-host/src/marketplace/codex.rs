use anyhow::Context;
use roder_api::marketplace::{
    MarketplaceKind, MarketplacePluginEntry, MarketplaceSource, PluginSource,
};
use roder_config::marketplaces::RawMarketplaceCatalog;

use super::{array_strings, author_name, component_hints, identity_key, risk_for, string_field};

pub fn normalize_catalog(
    catalog: &RawMarketplaceCatalog,
) -> anyhow::Result<Vec<MarketplacePluginEntry>> {
    catalog
        .plugin_manifests
        .iter()
        .map(|(path, manifest)| normalize_plugin(catalog, path, manifest.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::marketplace::{
        MarketplaceDescriptor, MarketplaceKind, MarketplaceSource, MarketplaceState,
        validate_plugin_entry,
    };

    #[test]
    fn shared_catalog_repository_does_not_become_plugin_identity() {
        let catalog = RawMarketplaceCatalog {
            marketplace: MarketplaceDescriptor {
                id: "codex-plugins".to_string(),
                kind: MarketplaceKind::Codex,
                display_name: "Codex Plugins".to_string(),
                source: MarketplaceSource::Github {
                    repo: "openai/plugins".to_string(),
                    ref_name: Some("main".to_string()),
                    catalog_path: None,
                    plugin_root: Some("plugins".to_string()),
                },
                homepage: None,
                owner_name: None,
                owner_email: None,
                description: None,
                is_default: true,
                enabled: true,
                state: MarketplaceState::Refreshed,
                last_refreshed_at: None,
                content_hash: None,
            },
            root: std::path::PathBuf::new(),
            value: serde_json::json!({}),
            plugin_manifests: vec![
                (
                    "alpaca/.codex-plugin/plugin.json".to_string(),
                    manifest("alpaca", "Alpaca", "https://alpaca.markets/"),
                ),
                (
                    "canva/.codex-plugin/plugin.json".to_string(),
                    manifest("canva", "Canva", "https://www.canva.com/"),
                ),
            ],
        };

        let entries = normalize_catalog(&catalog).expect("normalize codex catalog");

        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].repository.as_deref(),
            Some("https://github.com/openai/plugins")
        );
        assert_ne!(entries[0].identity_key, entries[1].identity_key);
        assert_ne!(
            entries[0].identity_key.repository.as_deref(),
            Some("https://github.com/openai/plugins")
        );
        assert_ne!(
            entries[1].identity_key.repository.as_deref(),
            Some("https://github.com/openai/plugins")
        );
        for entry in entries {
            validate_plugin_entry(&entry).expect("valid marketplace entry");
        }
    }

    fn manifest(name: &str, display_name: &str, homepage: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "repository": "https://github.com/openai/plugins",
            "homepage": homepage,
            "interface": {
                "displayName": display_name,
                "websiteURL": homepage
            }
        })
    }
}

fn normalize_plugin(
    catalog: &RawMarketplaceCatalog,
    path: &str,
    manifest: serde_json::Value,
) -> anyhow::Result<MarketplacePluginEntry> {
    let name = string_field(&manifest, "name").context("codex plugin manifest missing name")?;
    let interface = manifest
        .get("interface")
        .unwrap_or(&serde_json::Value::Null);
    let display_name = string_field(interface, "displayName").unwrap_or_else(|| name.clone());
    let source = PluginSource::MarketplacePath {
        marketplace_id: catalog.marketplace.id.clone(),
        path: path
            .trim_end_matches("/.codex-plugin/plugin.json")
            .trim_end_matches(".codex-plugin/plugin.json")
            .trim_end_matches('/')
            .to_string(),
    };
    let homepage = string_field(&manifest, "homepage")
        .or_else(|| string_field(interface, "websiteURL"))
        .or_else(|| catalog.marketplace.homepage.clone());
    let repository = string_field(&manifest, "repository");
    let author_name = author_name(&manifest).or_else(|| string_field(interface, "developerName"));
    let mut hints = component_hints(&manifest);
    hints.skills |= manifest.get("skills").is_some();
    hints.commands |= manifest.get("commands").is_some();
    hints.agents |= manifest.get("agents").is_some();
    hints.mcp_servers |= manifest.get("mcpServers").is_some();
    hints.apps |= manifest.get("app").is_some() || manifest.get("appJson").is_some();
    let identity_repository = repository.clone().filter(|repository| {
        !is_marketplace_catalog_repository(repository, &catalog.marketplace.source)
    });
    let identity_key = identity_key(
        &name,
        identity_repository,
        homepage.clone(),
        author_name.clone(),
        &source,
    );
    let risk = risk_for(&source, &hints);
    Ok(MarketplacePluginEntry {
        marketplace_id: catalog.marketplace.id.clone(),
        plugin_id: name,
        identity_key,
        display_name,
        description: string_field(&manifest, "description")
            .or_else(|| string_field(interface, "shortDescription")),
        kind: MarketplaceKind::Codex,
        version: string_field(&manifest, "version"),
        source,
        homepage,
        repository,
        author_name,
        category: string_field(interface, "category"),
        tags: array_strings(&manifest, "keywords"),
        component_hints: hints,
        capability_hints: array_strings(interface, "capabilities"),
        risk,
        raw_manifest: manifest,
    })
}

fn is_marketplace_catalog_repository(repository: &str, source: &MarketplaceSource) -> bool {
    let source_repository = match source {
        MarketplaceSource::Github { repo, .. } => repo,
        MarketplaceSource::Git { url, .. } => url,
        MarketplaceSource::HttpJson { .. } | MarketplaceSource::LocalPath { .. } => return false,
    };
    normalize_repository_reference(repository) == normalize_repository_reference(source_repository)
}

fn normalize_repository_reference(value: &str) -> String {
    let mut normalized = value.trim().trim_end_matches('/').to_ascii_lowercase();
    if let Some(stripped) = normalized.strip_suffix(".git") {
        normalized = stripped.to_string();
    }
    for prefix in [
        "https://www.github.com/",
        "https://github.com/",
        "http://www.github.com/",
        "http://github.com/",
        "git@github.com:",
    ] {
        if let Some(stripped) = normalized.strip_prefix(prefix) {
            return stripped.to_string();
        }
    }
    normalized
}
