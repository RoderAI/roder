use std::path::{Path, PathBuf};

use roder_api::marketplace::{MarketplaceDescriptor, MarketplaceSource};

pub fn resolve_local_source(marketplace: &MarketplaceDescriptor) -> anyhow::Result<PathBuf> {
    match &marketplace.source {
        MarketplaceSource::LocalPath { path } => Ok(PathBuf::from(path)),
        MarketplaceSource::Github { repo, .. }
        | MarketplaceSource::Git { url: repo, .. }
        | MarketplaceSource::HttpJson { url: repo } => {
            anyhow::bail!(
                "marketplace {} requires live fetch for remote source {repo}",
                marketplace.id
            )
        }
    }
}

pub fn marketplace_json_path(root: &Path, marketplace: &MarketplaceDescriptor) -> Option<PathBuf> {
    match &marketplace.source {
        MarketplaceSource::LocalPath { .. } => match marketplace.kind {
            roder_api::marketplace::MarketplaceKind::Claude => {
                Some(root.join(".claude-plugin").join("marketplace.json"))
            }
            roder_api::marketplace::MarketplaceKind::Cursor => {
                Some(root.join(".cursor-plugin").join("marketplace.json"))
            }
            roder_api::marketplace::MarketplaceKind::Roder
            | roder_api::marketplace::MarketplaceKind::Custom => {
                Some(root.join(".roder-plugin").join("marketplace.json"))
            }
            roder_api::marketplace::MarketplaceKind::Codex => None,
        },
        MarketplaceSource::Github { catalog_path, .. } => {
            catalog_path.as_ref().map(|p| root.join(p))
        }
        MarketplaceSource::Git { catalog_path, .. } => catalog_path.as_ref().map(|p| root.join(p)),
        MarketplaceSource::HttpJson { .. } => None,
    }
}

pub fn plugin_root(root: &Path, marketplace: &MarketplaceDescriptor) -> PathBuf {
    match &marketplace.source {
        MarketplaceSource::Github {
            plugin_root: Some(plugin_root),
            ..
        } => root.join(plugin_root),
        _ if marketplace.kind == roder_api::marketplace::MarketplaceKind::Codex => {
            root.join("plugins")
        }
        _ => root.to_path_buf(),
    }
}
