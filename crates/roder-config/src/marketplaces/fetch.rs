use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::marketplace::{MarketplaceDescriptor, MarketplaceKind, MarketplaceState};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::source::{marketplace_json_path, plugin_root, resolve_marketplace_source};
use super::store::{load_marketplace_store, save_marketplace_store};

#[derive(Debug, Clone)]
pub struct RawMarketplaceCatalog {
    pub marketplace: MarketplaceDescriptor,
    pub root: PathBuf,
    pub value: serde_json::Value,
    pub plugin_manifests: Vec<(String, serde_json::Value)>,
}

pub fn refresh_marketplace(marketplace_id: &str) -> anyhow::Result<RawMarketplaceCatalog> {
    let mut store = load_marketplace_store()?;
    let marketplace = store
        .marketplaces
        .iter()
        .find(|marketplace| marketplace.id == marketplace_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown marketplace {marketplace_id}"))?;
    let root = resolve_marketplace_source(&marketplace)?;
    let mut catalog = read_catalog_from_root(marketplace.clone(), &root)?;
    catalog.marketplace.state = MarketplaceState::Refreshed;
    catalog.marketplace.last_refreshed_at = Some(OffsetDateTime::now_utc());
    catalog.marketplace.content_hash = Some(hash_value(&catalog.value));
    store.upsert_marketplace(catalog.marketplace.clone());
    save_marketplace_store(&store)?;
    Ok(catalog)
}

pub fn read_catalog_from_root(
    marketplace: MarketplaceDescriptor,
    root: &Path,
) -> anyhow::Result<RawMarketplaceCatalog> {
    let value = match marketplace_json_path(root, &marketplace) {
        Some(path) if path.exists() => {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read marketplace catalog {}", path.display()))?;
            serde_json::from_str::<serde_json::Value>(&text)
                .with_context(|| format!("parse marketplace catalog {}", path.display()))?
        }
        Some(path) => anyhow::bail!("marketplace catalog {} does not exist", path.display()),
        None => serde_json::json!({
            "name": marketplace.id,
            "plugins": [],
        }),
    };
    let plugin_manifests = if marketplace.kind == MarketplaceKind::Codex {
        read_codex_plugin_manifests(&plugin_root(root, &marketplace))?
    } else {
        Vec::new()
    };
    Ok(RawMarketplaceCatalog {
        marketplace,
        root: root.to_path_buf(),
        value,
        plugin_manifests,
    })
}

fn read_codex_plugin_manifests(root: &Path) -> anyhow::Result<Vec<(String, serde_json::Value)>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let mut manifests = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path().join(".codex-plugin").join("plugin.json");
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read codex plugin manifest {}", path.display()))?;
        let value = serde_json::from_str::<serde_json::Value>(&text)
            .with_context(|| format!("parse codex plugin manifest {}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        manifests.push((relative, value));
    }
    manifests.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(manifests)
}

fn hash_value(value: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
