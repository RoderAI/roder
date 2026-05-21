use std::fs;
use std::path::{Path, PathBuf};

use roder_api::discovery::{
    DiscoveryCacheStatus, DiscoveryCatalog, DiscoveryCatalogItem, DiscoveryItemPromoted,
    DiscoveryItemRead, DiscoveryPromotionRecord, DiscoveryPromotionReused, DiscoveryPromotionState,
    DiscoveryWarmCacheHit,
};
use roder_api::events::RoderEvent;
use roder_extension_host::discovery_catalog::{
    DiscoveryCatalogBuildOptions, PromotionStore, build_file_backed_catalog_with_skills,
};
use roder_protocol::{
    DiscoveryGroupsParams, DiscoveryGroupsResult, DiscoveryPromoteParams, DiscoveryPromoteResult,
    DiscoveryPromotedClearParams, DiscoveryPromotedClearResult, DiscoveryPromotedListParams,
    DiscoveryPromotedListResult, DiscoveryReadPage, DiscoveryReadParams, DiscoveryReadResult,
    DiscoveryRefreshResult, DiscoverySearchParams, DiscoverySearchResult, JsonRpcError,
};
use time::OffsetDateTime;

use crate::server::AppServer;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 200;

impl AppServer {
    pub(crate) async fn handle_discovery_groups(
        &self,
        params: DiscoveryGroupsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let catalog = self
            .discovery_catalog(params.refresh.unwrap_or(false))
            .await?;
        let mut groups = catalog.groups.clone();
        groups.truncate(clamp_limit(params.limit));
        Ok(serde_json::to_value(DiscoveryGroupsResult {
            catalog_id: catalog.id,
            title: catalog.title,
            hidden_item_count: catalog.hidden_item_count,
            groups,
        })
        .unwrap())
    }

    pub(crate) async fn handle_discovery_search(
        &self,
        params: DiscoverySearchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params.query.trim().is_empty() {
            return Err(invalid_params("discovery/search requires query"));
        }
        let query = params.query.trim().to_ascii_lowercase();
        let catalog = self
            .discovery_catalog(params.refresh.unwrap_or(false))
            .await?;
        let limit = clamp_limit(params.limit);
        let items = catalog
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .filter(|item| item_matches(item, &query))
            .take(limit)
            .cloned()
            .collect();
        Ok(serde_json::to_value(DiscoverySearchResult {
            query: params.query,
            items,
        })
        .unwrap())
    }

    pub(crate) async fn handle_discovery_read(
        &self,
        params: DiscoveryReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let catalog = self
            .discovery_catalog(params.refresh.unwrap_or(false))
            .await?;
        let item = find_item(&catalog, &params.item_id)?;
        let page = read_item_page(
            &self.discovery_catalog_root(),
            &item,
            params.start_line.unwrap_or(1),
            clamp_limit(params.limit),
        )?;
        let promoted = params.promote.unwrap_or(true);
        if promoted {
            let thread_id = params.thread_id.unwrap_or_else(|| "app-server".to_string());
            let turn_id = params
                .turn_id
                .unwrap_or_else(|| "discovery-read".to_string());
            let record =
                self.promote_discovery_item(&item, thread_id.clone(), Some(turn_id.clone()))?;
            self.runtime
                .emit(RoderEvent::DiscoveryItemRead(DiscoveryItemRead {
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    item_id: item.id.clone(),
                    group_id: item.group_id.clone(),
                    promoted: true,
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
            self.emit_promotion_event(record).await;
        }
        Ok(serde_json::to_value(DiscoveryReadResult {
            item,
            page,
            promoted,
        })
        .unwrap())
    }

    pub(crate) async fn handle_discovery_refresh(&self) -> Result<serde_json::Value, JsonRpcError> {
        let result = self.rebuild_discovery_catalog().await?;
        Ok(serde_json::to_value(DiscoveryRefreshResult {
            catalog: result.catalog,
            catalog_root: result.catalog_root.display().to_string(),
            session_state_dir: result.session_state_dir.display().to_string(),
            written_files: result
                .written_files
                .into_iter()
                .map(|path| path.display().to_string())
                .collect(),
        })
        .unwrap())
    }

    pub(crate) async fn handle_discovery_promote(
        &self,
        params: DiscoveryPromoteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let catalog = self.discovery_catalog(false).await?;
        let item = find_item(&catalog, &params.item_id)?;
        let record = self.promote_discovery_item(&item, params.thread_id, params.turn_id)?;
        self.emit_promotion_event(record.clone()).await;
        Ok(serde_json::to_value(DiscoveryPromoteResult { record }).unwrap())
    }

    pub(crate) async fn handle_discovery_promoted_list(
        &self,
        params: DiscoveryPromotedListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut records = self.promotion_store().load().map_err(internal_error)?;
        if let Some(thread_id) = params.thread_id {
            records.retain(|record| record.thread_id == thread_id);
        }
        Ok(serde_json::to_value(DiscoveryPromotedListResult { records }).unwrap())
    }

    pub(crate) async fn handle_discovery_promoted_clear(
        &self,
        params: DiscoveryPromotedClearParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = self.promotion_store();
        let mut records = store.load().map_err(internal_error)?;
        let original_len = records.len();
        records.retain(|record| {
            let thread_matches = params
                .thread_id
                .as_ref()
                .is_none_or(|thread_id| &record.thread_id == thread_id);
            let item_matches = params
                .item_id
                .as_ref()
                .is_none_or(|item_id| &record.item_id == item_id);
            !(thread_matches && item_matches)
        });
        let cleared = original_len.saturating_sub(records.len());
        store.save(&records).map_err(internal_error)?;
        Ok(serde_json::to_value(DiscoveryPromotedClearResult { cleared }).unwrap())
    }

    async fn discovery_catalog(&self, refresh: bool) -> Result<DiscoveryCatalog, JsonRpcError> {
        if refresh || !self.discovery_catalog_root().join("index.json").exists() {
            return Ok(self.rebuild_discovery_catalog().await?.catalog);
        }
        let text = fs::read_to_string(self.discovery_catalog_root().join("index.json"))
            .map_err(internal_error)?;
        serde_json::from_str(&text).map_err(internal_error)
    }

    async fn rebuild_discovery_catalog(
        &self,
    ) -> Result<roder_extension_host::discovery_catalog::DiscoveryCatalogBuildResult, JsonRpcError>
    {
        let workspace = self.runtime.workspace();
        let workflow =
            roder_config::scan_workflow_imports(roder_config::WorkflowScanOptions::new(&workspace));
        let skills = self
            .runtime
            .skills_snapshot()
            .await
            .skills()
            .iter()
            .map(|skill| skill.descriptor.clone())
            .collect::<Vec<_>>();
        build_file_backed_catalog_with_skills(
            self.runtime.registry(),
            &workflow.items,
            &skills,
            &DiscoveryCatalogBuildOptions::new(
                self.discovery_catalog_root(),
                self.discovery_session_state_dir(),
            ),
        )
        .map_err(internal_error)
    }

    fn discovery_catalog_root(&self) -> PathBuf {
        if let Ok(path) = std::env::var("RODER_DISCOVERY_CATALOG_DIR") {
            return PathBuf::from(path);
        }
        default_roder_dir().join("discovery")
    }

    fn discovery_session_state_dir(&self) -> PathBuf {
        if let Ok(path) = std::env::var("RODER_DISCOVERY_SESSION_DIR") {
            return PathBuf::from(path);
        }
        default_roder_dir().join("sessions").join("discovery-state")
    }

    fn promotion_store(&self) -> PromotionStore {
        PromotionStore::new(self.discovery_session_state_dir())
    }

    fn promote_discovery_item(
        &self,
        item: &DiscoveryCatalogItem,
        thread_id: String,
        turn_id: Option<String>,
    ) -> Result<DiscoveryPromotionRecord, JsonRpcError> {
        let store = self.promotion_store();
        let mut records = store.load().map_err(internal_error)?;
        let now = OffsetDateTime::now_utc();
        let mut record = DiscoveryPromotionRecord {
            item_id: item.id.clone(),
            group_id: item.group_id.clone(),
            thread_id: thread_id.clone(),
            turn_id,
            promotion: DiscoveryPromotionState::Promoted,
            cache_status: DiscoveryCacheStatus::Warm,
            reused_count: 0,
            timestamp: now,
        };
        if let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.item_id == item.id && existing.thread_id == thread_id)
        {
            existing.promotion = DiscoveryPromotionState::Reused;
            existing.cache_status = DiscoveryCacheStatus::Hit;
            existing.reused_count += 1;
            existing.turn_id = record.turn_id.clone();
            existing.timestamp = now;
            record = existing.clone();
        } else {
            records.push(record.clone());
        }
        store.save(&records).map_err(internal_error)?;
        Ok(record)
    }

    async fn emit_promotion_event(&self, record: DiscoveryPromotionRecord) {
        let event = match record.promotion {
            DiscoveryPromotionState::Reused => {
                RoderEvent::DiscoveryPromotionReused(DiscoveryPromotionReused { record })
            }
            DiscoveryPromotionState::WarmCacheHit => {
                RoderEvent::DiscoveryWarmCacheHit(DiscoveryWarmCacheHit { record })
            }
            _ => RoderEvent::DiscoveryItemPromoted(DiscoveryItemPromoted { record }),
        };
        self.runtime.emit(event).await;
    }
}

fn find_item(
    catalog: &DiscoveryCatalog,
    item_id: &str,
) -> Result<DiscoveryCatalogItem, JsonRpcError> {
    catalog
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|item| item.id == item_id)
        .cloned()
        .ok_or_else(|| invalid_params(format!("discovery item not found: {item_id}")))
}

fn read_item_page(
    root: &Path,
    item: &DiscoveryCatalogItem,
    start_line: usize,
    limit: usize,
) -> Result<DiscoveryReadPage, JsonRpcError> {
    let Some(schema) = item.schema.as_ref() else {
        return Ok(DiscoveryReadPage {
            text: "item has no detailed schema or instruction file".to_string(),
            start_line: 1,
            end_line: 1,
            total_lines: 1,
            truncated: false,
        });
    };
    let path = safe_catalog_path(root, &schema.uri)?;
    let text = fs::read_to_string(path).map_err(internal_error)?;
    let lines = text.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let start_line = start_line.max(1);
    let start_index = start_line.saturating_sub(1).min(total_lines);
    let end_index = (start_index + limit).min(total_lines);
    Ok(DiscoveryReadPage {
        text: lines[start_index..end_index].join("\n"),
        start_line,
        end_line: end_index,
        total_lines,
        truncated: end_index < total_lines,
    })
}

fn safe_catalog_path(root: &Path, relative: &str) -> Result<PathBuf, JsonRpcError> {
    if relative.contains("..") || relative.starts_with('/') {
        return Err(invalid_params(
            "discovery schema path escapes catalog root".to_string(),
        ));
    }
    Ok(root.join(relative))
}

fn item_matches(item: &DiscoveryCatalogItem, query: &str) -> bool {
    [
        item.id.as_str(),
        item.name.as_str(),
        item.title.as_str(),
        item.description.as_deref().unwrap_or_default(),
        &item.tags.join(" "),
        &item.hints.join(" "),
    ]
    .join("\n")
    .to_ascii_lowercase()
    .contains(query)
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn default_roder_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".roder")
}

fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}
