use roder_api::marketplace::{
    InstalledPluginRecord, MarketplaceInstallState, MarketplacePluginEntry,
    MarketplacePluginVariant, variant_key,
};
use time::OffsetDateTime;

pub fn preview_plugin_install(entry: &MarketplacePluginEntry) -> serde_json::Value {
    serde_json::json!({
        "marketplaceId": entry.marketplace_id,
        "pluginId": entry.plugin_id,
        "displayName": entry.display_name,
        "identityKey": entry.identity_key,
        "source": entry.source,
        "componentHints": entry.component_hints,
        "capabilityHints": entry.capability_hints,
        "risk": entry.risk,
        "rawManifest": entry.raw_manifest,
    })
}

pub fn install_plugin_variant(
    entry: &MarketplacePluginEntry,
) -> anyhow::Result<InstalledPluginRecord> {
    let install_path = roder_config::marketplaces::cache::record_cached_marker(entry)?;
    let content_hash = roder_config::marketplaces::cache::content_hash(&entry.raw_manifest);
    Ok(InstalledPluginRecord {
        marketplace_id: entry.marketplace_id.clone(),
        plugin_id: entry.plugin_id.clone(),
        identity_key: entry.identity_key.clone(),
        variant_key: variant_key(&entry.marketplace_id, &entry.plugin_id),
        install_path: install_path.display().to_string(),
        version: entry.version.clone(),
        content_hash: Some(content_hash),
        state: MarketplaceInstallState::Installed,
        installed_at: OffsetDateTime::now_utc(),
    })
}

pub fn variant_matches_entry(
    variant: &MarketplacePluginVariant,
    entry: &MarketplacePluginEntry,
) -> bool {
    variant.marketplace_id == entry.marketplace_id && variant.plugin_id == entry.plugin_id
}
