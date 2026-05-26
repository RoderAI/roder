use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::discovery::{
    DiscoveryCacheStatus, DiscoveryCatalog, DiscoveryCatalogItem, DiscoveryPromotionRecord,
    DiscoveryPromotionState,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;

const DEFAULT_LIMIT: usize = 50;
const MAX_READ_LINES: usize = 200;

pub fn register(registry: &mut ToolRegistry) -> anyhow::Result<()> {
    registry.register(Arc::new(DiscoveryListTool))?;
    registry.register(Arc::new(DiscoverySearchTool))?;
    registry.register(Arc::new(DiscoveryReadTool))
}

struct DiscoveryListTool;

#[async_trait::async_trait]
impl ToolExecutor for DiscoveryListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "discovery.list".to_string(),
            description:
                "List lazy capability discovery catalog groups and compact item summaries."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
                },
                "x-roder": {
                    "retrievalMode": "discovery",
                    "retrievalMetadata": true
                }
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ListArgs>(&call)?;
        let catalog = load_catalog()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 200);
        let summaries = compact_items(&catalog, limit);
        let text = if summaries.is_empty() {
            "discovery catalog is empty".to_string()
        } else {
            summaries.join("\n")
        };
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "catalogId": catalog.id,
                "groupCount": catalog.groups.len(),
                "hiddenItemCount": catalog.hidden_item_count,
                "retrieval_mode": "discovery",
                "items": summaries,
            }),
            is_error: false,
        })
    }
}

struct DiscoverySearchTool;

#[async_trait::async_trait]
impl ToolExecutor for DiscoverySearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "discovery.search".to_string(),
            description: "Search lazy capability discovery catalog items by name, title, description, tag, or hint."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
                },
                "required": ["query"],
                "x-roder": {
                    "retrievalMode": "discovery",
                    "retrievalMetadata": true
                }
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<SearchArgs>(&call)?;
        let query = args.query.trim().to_ascii_lowercase();
        if query.is_empty() {
            anyhow::bail!("query is required");
        }
        let catalog = load_catalog()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 200);
        let matches = catalog
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .filter(|item| item_matches(item, &query))
            .take(limit)
            .map(item_summary)
            .collect::<Vec<_>>();
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text: if matches.is_empty() {
                format!("no discovery items matched {query:?}")
            } else {
                matches.join("\n")
            },
            data: json!({
                "query": query,
                "retrieval_mode": "discovery",
                "matches": matches,
            }),
            is_error: false,
        })
    }
}

struct DiscoveryReadTool;

#[async_trait::async_trait]
impl ToolExecutor for DiscoveryReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "discovery.read".to_string(),
            description: "Read and promote one lazy discovery catalog item by id, including bounded schema or instruction content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "item_id": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "promote": { "type": "boolean" }
                },
                "required": ["item_id"],
                "x-roder": {
                    "retrievalMode": "promotion",
                    "retrievalMetadata": true
                }
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ReadArgs>(&call)?;
        let catalog = load_catalog()?;
        let root = catalog_root()?;
        let item = find_item(&catalog, &args.item_id)?;
        let limit = args
            .limit
            .unwrap_or(MAX_READ_LINES)
            .clamp(1, MAX_READ_LINES);
        let start_line = args.start_line.unwrap_or(1).max(1);
        let content = read_item_content(&root, &item, start_line, limit)?;
        let promoted = args.promote.unwrap_or(true);
        if promoted {
            record_promotion(&ctx, &item)?;
        }
        let text = format!(
            "{}\n{}\n{}",
            item_summary(&item),
            if promoted {
                "promotion: recorded for this thread"
            } else {
                "promotion: skipped"
            },
            content.text
        );
        Ok(ToolResult {
            id: call.id,
            name: call.name,
            text,
            data: json!({
                "item": item,
                "promoted": promoted,
                "retrieval_mode": "promotion",
                "page": content,
            }),
            is_error: false,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ListArgs {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    item_id: String,
    start_line: Option<usize>,
    limit: Option<usize>,
    promote: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadPage {
    text: String,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    truncated: bool,
}

fn parse<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> anyhow::Result<T> {
    Ok(serde_json::from_value(call.arguments.clone())?)
}

fn load_catalog() -> anyhow::Result<DiscoveryCatalog> {
    let path = catalog_root()?.join("index.json");
    Ok(serde_json::from_str(&fs::read_to_string(&path)?)?)
}

fn catalog_root() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("RODER_DISCOVERY_CATALOG_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = roder_data_dir() {
        return Ok(path.join("discovery"));
    }
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
        .join(".roder")
        .join("discovery"))
}

fn promotion_state_dir() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("RODER_DISCOVERY_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = roder_data_dir() {
        return Ok(path.join("threads").join("discovery-state"));
    }
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
        .join(".roder")
        .join("threads")
        .join("discovery-state"))
}

fn roder_data_dir() -> Option<PathBuf> {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
}

fn compact_items(catalog: &DiscoveryCatalog, limit: usize) -> Vec<String> {
    catalog
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .take(limit)
        .map(item_summary)
        .collect()
}

fn item_summary(item: &DiscoveryCatalogItem) -> String {
    format!(
        "- {} [{} {:?}/{:?}] {}",
        item.id,
        item.source.id,
        item.status,
        item.promotion,
        item.description.as_deref().unwrap_or(&item.title)
    )
}

fn item_matches(item: &DiscoveryCatalogItem, query: &str) -> bool {
    let haystack = [
        item.id.as_str(),
        item.name.as_str(),
        item.title.as_str(),
        item.description.as_deref().unwrap_or_default(),
        &item.tags.join(" "),
        &item.hints.join(" "),
    ]
    .join("\n")
    .to_ascii_lowercase();
    haystack.contains(query)
}

fn find_item(catalog: &DiscoveryCatalog, item_id: &str) -> anyhow::Result<DiscoveryCatalogItem> {
    catalog
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .find(|item| item.id == item_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("discovery item not found: {item_id}"))
}

fn read_item_content(
    root: &Path,
    item: &DiscoveryCatalogItem,
    start_line: usize,
    limit: usize,
) -> anyhow::Result<ReadPage> {
    let Some(schema) = item.schema.as_ref() else {
        return Ok(ReadPage {
            text: "item has no detailed schema or instruction file".to_string(),
            start_line: 1,
            end_line: 1,
            total_lines: 1,
            truncated: false,
        });
    };
    let path = safe_catalog_path(root, &schema.uri)?;
    let text = fs::read_to_string(path)?;
    let lines = text.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let start_index = start_line.saturating_sub(1).min(total_lines);
    let end_index = (start_index + limit).min(total_lines);
    Ok(ReadPage {
        text: lines[start_index..end_index].join("\n"),
        start_line,
        end_line: end_index,
        total_lines,
        truncated: end_index < total_lines,
    })
}

fn safe_catalog_path(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    if relative.contains("..") || relative.starts_with('/') {
        anyhow::bail!("discovery schema path escapes catalog root");
    }
    Ok(root.join(relative))
}

fn record_promotion(ctx: &ToolExecutionContext, item: &DiscoveryCatalogItem) -> anyhow::Result<()> {
    let path = promotion_state_dir()?
        .join("discovery")
        .join("promotions.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut records = if path.exists() {
        serde_json::from_str::<Vec<DiscoveryPromotionRecord>>(&fs::read_to_string(&path)?)?
    } else {
        Vec::new()
    };
    if let Some(existing) = records
        .iter_mut()
        .find(|record| record.item_id == item.id && record.thread_id == ctx.thread_id)
    {
        existing.promotion = DiscoveryPromotionState::Reused;
        existing.cache_status = DiscoveryCacheStatus::Hit;
        existing.reused_count += 1;
        existing.turn_id = Some(ctx.turn_id.clone());
        existing.timestamp = OffsetDateTime::now_utc();
    } else {
        records.push(DiscoveryPromotionRecord {
            item_id: item.id.clone(),
            group_id: item.group_id.clone(),
            thread_id: ctx.thread_id.clone(),
            turn_id: Some(ctx.turn_id.clone()),
            promotion: DiscoveryPromotionState::Promoted,
            cache_status: DiscoveryCacheStatus::Warm,
            reused_count: 0,
            timestamp: OffsetDateTime::now_utc(),
        });
    }
    fs::write(path, serde_json::to_string_pretty(&records)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "discovery/tests.rs"]
mod tests;
