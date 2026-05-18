use anyhow::Context;
use roder_api::marketplace::{MarketplaceKind, MarketplacePluginEntry, PluginSource};
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
    let identity_key = identity_key(
        &name,
        repository.clone(),
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
