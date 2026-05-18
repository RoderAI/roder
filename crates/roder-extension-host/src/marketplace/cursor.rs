use anyhow::Context;
use roder_api::marketplace::{MarketplaceKind, MarketplacePluginEntry, PluginSource};
use roder_config::marketplaces::RawMarketplaceCatalog;

use super::{
    array_strings, component_hints, identity_key, risk_for, source_from_value, string_field,
};

pub fn normalize_catalog(
    catalog: &RawMarketplaceCatalog,
) -> anyhow::Result<Vec<MarketplacePluginEntry>> {
    let plugins = catalog
        .value
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    plugins
        .into_iter()
        .map(|plugin| normalize_plugin(catalog, plugin))
        .collect()
}

fn normalize_plugin(
    catalog: &RawMarketplaceCatalog,
    plugin: serde_json::Value,
) -> anyhow::Result<MarketplacePluginEntry> {
    let name = string_field(&plugin, "name").context("cursor plugin entry missing name")?;
    let plugin_id = string_field(&plugin, "id").unwrap_or_else(|| name.clone());
    let source = source_from_value(
        &catalog.marketplace.id,
        plugin.get("source"),
        Some(name.clone()),
    );
    let repository = string_field(&plugin, "repository").or_else(|| match &source {
        PluginSource::MarketplacePath { .. } => catalog.marketplace.homepage.clone(),
        PluginSource::Git { url, .. } | PluginSource::Http { url, .. } => Some(url.clone()),
        _ => None,
    });
    let homepage = string_field(&plugin, "homepage").or_else(|| repository.clone());
    let mut hints = component_hints(&plugin);
    hints.skills = true;
    hints.rules |= plugin.get("rules").is_some();
    hints.mcp_servers |= plugin.get("mcp").is_some() || plugin.get("mcpServers").is_some();
    let identity_key = identity_key(&name, repository.clone(), homepage.clone(), None, &source);
    let risk = risk_for(&source, &hints);
    Ok(MarketplacePluginEntry {
        marketplace_id: catalog.marketplace.id.clone(),
        plugin_id,
        identity_key,
        display_name: name,
        description: string_field(&plugin, "description"),
        kind: MarketplaceKind::Cursor,
        version: string_field(&plugin, "version"),
        source,
        homepage,
        repository,
        author_name: catalog.marketplace.owner_name.clone(),
        category: None,
        tags: array_strings(&plugin, "tags"),
        component_hints: hints,
        capability_hints: Vec::new(),
        risk,
        raw_manifest: plugin,
    })
}
