use std::path::PathBuf;

use anyhow::Context;
use roder_api::marketplace::{MarketplacePluginEntry, variant_key};
use sha2::{Digest, Sha256};

pub fn marketplace_cache_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("RODER_MARKETPLACE_CACHE_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(crate::config_dir().join("plugins").join("cache"))
}

pub fn install_path_for(entry: &MarketplacePluginEntry) -> anyhow::Result<PathBuf> {
    let version_or_hash = entry
        .version
        .clone()
        .unwrap_or_else(|| short_hash(&entry.raw_manifest.to_string()));
    Ok(marketplace_cache_dir()?
        .join(&entry.marketplace_id)
        .join(&entry.plugin_id)
        .join(version_or_hash))
}

pub fn record_cached_marker(entry: &MarketplacePluginEntry) -> anyhow::Result<PathBuf> {
    let path = install_path_for(entry)?;
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create plugin cache {}", path.display()))?;
    let marker = serde_json::json!({
        "variantKey": variant_key(&entry.marketplace_id, &entry.plugin_id),
        "marketplaceId": entry.marketplace_id,
        "pluginId": entry.plugin_id,
        "manifest": entry.raw_manifest,
    });
    std::fs::write(
        path.join("install-preview.json"),
        serde_json::to_string_pretty(&marker)?,
    )
    .with_context(|| format!("write plugin cache marker {}", path.display()))?;
    Ok(path)
}

pub fn content_hash(value: &serde_json::Value) -> String {
    short_hash(&value.to_string())
}

fn short_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_digest(hasher.finalize())[..16].to_string()
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
