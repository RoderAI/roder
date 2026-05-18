use anyhow::Context;
use roder_api::marketplace::{MarketplaceKind, MarketplacePluginEntry};
use roder_config::marketplaces::RawMarketplaceCatalog;

use super::{
    array_strings, author_name, component_hints, identity_key, risk_for, source_from_value,
    string_field,
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
    let name = string_field(&plugin, "name").context("claude plugin entry missing name")?;
    let display_name = string_field(&plugin, "displayName").unwrap_or_else(|| name.clone());
    let source = source_from_value(
        &catalog.marketplace.id,
        plugin.get("source"),
        Some(name.clone()),
    );
    let homepage = string_field(&plugin, "homepage");
    let repository = string_field(&plugin, "repository");
    let author_name = author_name(&plugin);
    let mut hints = component_hints(&plugin);
    hints.skills |= plugin.get("skills").is_some() || plugin.get("skill").is_some();
    hints.commands |= plugin.get("commands").is_some();
    hints.agents |= plugin.get("agents").is_some();
    hints.mcp_servers |= plugin.get("mcpServers").is_some();
    hints.hooks |= plugin.get("hooks").is_some();
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
        description: string_field(&plugin, "description"),
        kind: MarketplaceKind::Claude,
        version: string_field(&plugin, "version"),
        source,
        homepage,
        repository,
        author_name,
        category: string_field(&plugin, "category"),
        tags: array_strings(&plugin, "tags"),
        component_hints: hints,
        capability_hints: array_strings(&plugin, "capabilities"),
        risk,
        raw_manifest: plugin,
    })
}
