use std::fs;
use std::path::{Path, PathBuf};

use roder_api::discovery::{DiscoveryCatalog, DiscoveryCatalogGroup, DiscoveryPromotionRecord};
use roder_api::extension::ExtensionRegistry;
use roder_api::workflow::WorkflowImportItem;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

mod build;
mod state;

#[cfg(test)]
mod tests;

pub use state::PromotionStore;

#[derive(Debug, Clone)]
pub struct DiscoveryCatalogBuildOptions {
    pub catalog_root: PathBuf,
    pub session_state_dir: PathBuf,
}

impl DiscoveryCatalogBuildOptions {
    pub fn new(catalog_root: impl Into<PathBuf>, session_state_dir: impl Into<PathBuf>) -> Self {
        Self {
            catalog_root: catalog_root.into(),
            session_state_dir: session_state_dir.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCatalogBuildResult {
    pub catalog: DiscoveryCatalog,
    pub catalog_root: PathBuf,
    pub session_state_dir: PathBuf,
    pub written_files: Vec<PathBuf>,
}

pub fn build_file_backed_catalog(
    registry: &ExtensionRegistry,
    workflow_items: &[WorkflowImportItem],
    options: &DiscoveryCatalogBuildOptions,
) -> anyhow::Result<DiscoveryCatalogBuildResult> {
    fs::create_dir_all(&options.catalog_root)?;
    fs::create_dir_all(&options.session_state_dir)?;

    let promoted = PromotionStore::new(&options.session_state_dir).load()?;
    let built_at = OffsetDateTime::now_utc();
    let mut groups = Vec::new();
    groups.extend(build::tool_groups(
        registry,
        &options.catalog_root,
        &promoted,
    )?);
    groups.extend(build::workflow_groups(
        workflow_items,
        &options.catalog_root,
        &promoted,
    )?);
    groups.extend(build::subagent_groups(
        registry,
        &options.catalog_root,
        &promoted,
    )?);
    groups.sort_by(|left, right| left.id.cmp(&right.id));

    let hidden_item_count = groups.iter().map(|group| group.hidden_item_count).sum();
    let catalog = DiscoveryCatalog {
        id: "default".to_string(),
        title: "Roder capability discovery".to_string(),
        description: Some(
            "Lazy discovery catalog for tools, workflow imports, subagents, and artifact surfaces."
                .to_string(),
        ),
        groups,
        hidden_item_count,
        built_at: Some(built_at),
    };

    let mut written_files = write_catalog(&catalog, &options.catalog_root)?;
    written_files.push(PromotionStore::new(&options.session_state_dir).ensure()?);
    Ok(DiscoveryCatalogBuildResult {
        catalog,
        catalog_root: options.catalog_root.clone(),
        session_state_dir: options.session_state_dir.clone(),
        written_files,
    })
}

fn write_catalog(catalog: &DiscoveryCatalog, root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let catalog_path = root.join("index.json");
    build::write_json(&catalog_path, catalog)?;
    written.push(catalog_path);
    for group in &catalog.groups {
        written.push(write_catalog_group(root, group)?);
    }
    Ok(written)
}

pub fn write_catalog_group(
    root: impl AsRef<Path>,
    group: &DiscoveryCatalogGroup,
) -> anyhow::Result<PathBuf> {
    let group_path = root
        .as_ref()
        .join(build::source_kind_segment(&group.source.kind))
        .join(build::safe_segment(&group.source.id))
        .join("index.json");
    build::write_json(&group_path, group)?;
    Ok(group_path)
}

pub(crate) fn apply_promoted_state(
    item: &mut roder_api::discovery::DiscoveryCatalogItem,
    promoted: &[DiscoveryPromotionRecord],
) {
    use roder_api::discovery::{
        DiscoveryCacheStatus, DiscoveryLifecycleState, DiscoveryPromotionState,
    };

    if let Some(record) = promoted.iter().find(|record| record.item_id == item.id) {
        item.lifecycle = match record.promotion {
            DiscoveryPromotionState::WarmCacheHit => DiscoveryLifecycleState::WarmCached,
            DiscoveryPromotionState::Reused => DiscoveryLifecycleState::Reused,
            DiscoveryPromotionState::Expired => DiscoveryLifecycleState::Expired,
            _ => DiscoveryLifecycleState::Promoted,
        };
        item.promotion = record.promotion.clone();
        item.cache_status = match record.cache_status {
            DiscoveryCacheStatus::Hit => DiscoveryCacheStatus::Hit,
            _ => record.cache_status.clone(),
        };
    }
}

pub(crate) fn group(
    prefix: &str,
    id: &str,
    kind: roder_api::discovery::DiscoverySourceKind,
    description: &str,
    items: Vec<roder_api::discovery::DiscoveryCatalogItem>,
) -> DiscoveryCatalogGroup {
    use roder_api::discovery::{
        DiscoveryAuthState, DiscoveryCatalogSource, DiscoveryItemStatus, DiscoveryPromotionState,
        DiscoveryRedaction,
    };

    let item_count = items.len() as u64;
    let hidden_item_count = items
        .iter()
        .filter(|item| item.promotion == DiscoveryPromotionState::NotPromoted)
        .count() as u64;
    DiscoveryCatalogGroup {
        id: format!("{prefix}:{}", build::safe_segment(id)),
        catalog_id: "default".to_string(),
        source: DiscoveryCatalogSource {
            kind,
            id: id.to_string(),
            display_name: id.to_string(),
            origin: None,
            auth_state: DiscoveryAuthState::NotRequired,
            redaction: DiscoveryRedaction::none(),
        },
        title: id.to_string(),
        description: Some(description.to_string()),
        status: DiscoveryItemStatus::Available,
        item_count,
        hidden_item_count,
        items,
        last_refreshed_at: None,
    }
}
