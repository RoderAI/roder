use std::path::PathBuf;

use anyhow::Context;
use roder_api::marketplace::{InstalledPluginRecord, MarketplaceDescriptor};
use serde::{Deserialize, Serialize};

use super::defaults::default_marketplaces;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceStore {
    #[serde(default)]
    pub marketplaces: Vec<MarketplaceDescriptor>,
    #[serde(default)]
    pub installed_plugins: Vec<InstalledPluginRecord>,
}

impl MarketplaceStore {
    pub fn with_baked_in_defaults(mut self) -> Self {
        for default in default_marketplaces() {
            if !self
                .marketplaces
                .iter()
                .any(|marketplace| marketplace.id == default.id)
            {
                self.marketplaces.push(default);
            }
        }
        self.marketplaces.sort_by(|a, b| a.id.cmp(&b.id));
        self
    }

    pub fn upsert_marketplace(&mut self, marketplace: MarketplaceDescriptor) {
        self.marketplaces
            .retain(|existing| existing.id != marketplace.id);
        self.marketplaces.push(marketplace);
        self.marketplaces.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn remove_marketplace(&mut self, marketplace_id: &str) -> bool {
        let before = self.marketplaces.len();
        self.marketplaces
            .retain(|marketplace| marketplace.id != marketplace_id);
        before != self.marketplaces.len()
    }

    pub fn upsert_installed_plugin(&mut self, plugin: InstalledPluginRecord) {
        self.installed_plugins
            .retain(|existing| existing.variant_key != plugin.variant_key);
        self.installed_plugins.push(plugin);
        self.installed_plugins
            .sort_by(|a, b| a.variant_key.cmp(&b.variant_key));
    }
}

pub fn marketplace_store_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("RODER_MARKETPLACES_PATH") {
        return Ok(PathBuf::from(path));
    }
    Ok(crate::config_dir().join("marketplaces.json"))
}

pub fn load_marketplace_store() -> anyhow::Result<MarketplaceStore> {
    let path = marketplace_store_path()?;
    if !path.exists() {
        return Ok(MarketplaceStore::default().with_baked_in_defaults());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read marketplace store {}", path.display()))?;
    let store = serde_json::from_str::<MarketplaceStore>(&text)
        .with_context(|| format!("parse marketplace store {}", path.display()))?;
    Ok(store.with_baked_in_defaults())
}

pub fn save_marketplace_store(store: &MarketplaceStore) -> anyhow::Result<()> {
    let path = marketplace_store_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create marketplace store dir {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, text)
        .with_context(|| format!("write marketplace store {}", path.display()))?;
    Ok(())
}
