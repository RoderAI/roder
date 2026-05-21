use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use roder_api::discovery::{
    DiscoveryAuthState, DiscoveryCacheStatus, DiscoveryCatalog, DiscoveryCatalogGroup,
    DiscoveryCatalogItem, DiscoveryCatalogSource, DiscoveryItemStatus, DiscoveryLifecycleState,
    DiscoveryPromotionState, DiscoveryRedaction, DiscoverySchemaFormat, DiscoverySchemaReference,
    DiscoverySourceKind,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry, ToolResult};
use serde_json::json;
use time::OffsetDateTime;

use super::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn discovery_tools_list_search_read_and_record_promotion() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("tools");
    let session = temp_dir("session");
    write_fixture_catalog(&root);
    unsafe {
        std::env::set_var("RODER_DISCOVERY_CATALOG_DIR", &root);
        std::env::set_var("RODER_DISCOVERY_SESSION_DIR", &session);
    }

    let mut registry = ToolRegistry::default();
    register(&mut registry).unwrap();
    let ctx = ToolExecutionContext::new("thread-a", "turn-a", PolicyMode::Default);
    let list = run(&registry, ctx.clone(), "discovery.list", json!({})).await;
    assert!(list.text.contains("tool:builtin/grep"));
    let search = run(
        &registry,
        ctx.clone(),
        "discovery.search",
        json!({ "query": "grep" }),
    )
    .await;
    assert!(search.text.contains("tool:builtin/grep"));
    let read = run(
        &registry,
        ctx,
        "discovery.read",
        json!({ "item_id": "tool:builtin/grep" }),
    )
    .await;
    assert!(read.text.contains("\"query\""));
    assert!(session.join("discovery/promotions.json").exists());

    unsafe {
        std::env::remove_var("RODER_DISCOVERY_CATALOG_DIR");
        std::env::remove_var("RODER_DISCOVERY_SESSION_DIR");
    }
}

async fn run(
    registry: &ToolRegistry,
    ctx: ToolExecutionContext,
    name: &str,
    arguments: serde_json::Value,
) -> ToolResult {
    registry
        .get(name)
        .unwrap()
        .execute(
            ctx,
            ToolCall {
                id: format!("call-{name}"),
                name: name.to_string(),
                raw_arguments: arguments.to_string(),
                arguments,
                thread_id: "thread-a".to_string(),
                turn_id: "turn-a".to_string(),
            },
        )
        .await
        .unwrap()
}

fn write_fixture_catalog(root: &Path) {
    fs::create_dir_all(root.join("tools/builtin")).unwrap();
    fs::write(
        root.join("tools/builtin/grep.schema.json"),
        serde_json::to_string_pretty(&json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        }))
        .unwrap(),
    )
    .unwrap();
    let item = DiscoveryCatalogItem {
        id: "tool:builtin/grep".to_string(),
        group_id: "tools:builtin".to_string(),
        source: DiscoveryCatalogSource {
            kind: DiscoverySourceKind::InternalTools,
            id: "builtin".to_string(),
            display_name: "Builtins".to_string(),
            origin: None,
            auth_state: DiscoveryAuthState::NotRequired,
            redaction: DiscoveryRedaction::none(),
        },
        name: "grep".to_string(),
        title: "grep".to_string(),
        description: Some("Search files".to_string()),
        status: DiscoveryItemStatus::Available,
        lifecycle: DiscoveryLifecycleState::Discovered,
        promotion: DiscoveryPromotionState::NotPromoted,
        cache_status: DiscoveryCacheStatus::Cold,
        schema: Some(DiscoverySchemaReference {
            format: DiscoverySchemaFormat::JsonSchema,
            uri: "tools/builtin/grep.schema.json".to_string(),
            content_hash: None,
            byte_count: None,
            redaction: DiscoveryRedaction::none(),
        }),
        tags: vec!["tool".to_string()],
        hints: vec!["read before use".to_string()],
        redaction: DiscoveryRedaction::none(),
        last_refreshed_at: None,
    };
    let group = DiscoveryCatalogGroup {
        id: "tools:builtin".to_string(),
        catalog_id: "default".to_string(),
        source: item.source.clone(),
        title: "Builtins".to_string(),
        description: None,
        status: DiscoveryItemStatus::Available,
        item_count: 1,
        hidden_item_count: 1,
        items: vec![item],
        last_refreshed_at: None,
    };
    let catalog = DiscoveryCatalog {
        id: "default".to_string(),
        title: "fixture".to_string(),
        description: None,
        groups: vec![group],
        hidden_item_count: 1,
        built_at: None,
    };
    fs::write(
        root.join("index.json"),
        serde_json::to_string_pretty(&catalog).unwrap(),
    )
    .unwrap();
}

fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "roder-discovery-tools-{name}-{}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
