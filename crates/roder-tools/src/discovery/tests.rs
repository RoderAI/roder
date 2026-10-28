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

struct EnvGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(name: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(name);
        unsafe { std::env::set_var(name, value) };
        Self { name, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

#[test]
fn discovery_tools_advertise_retrieval_metadata() {
    let mut registry = ToolRegistry::default();
    register(&mut registry).unwrap();

    assert_eq!(
        registry.get("discovery.list").unwrap().spec().parameters["x-roder"]["retrievalMode"],
        "discovery"
    );
    assert_eq!(
        registry.get("discovery.search").unwrap().spec().parameters["x-roder"]["retrievalMode"],
        "discovery"
    );
    assert_eq!(
        registry.get("discovery.read").unwrap().spec().parameters["x-roder"]["retrievalMode"],
        "promotion"
    );
}

#[tokio::test]
async fn discovery_tools_list_search_read_and_record_promotion() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("tools");
    let state = temp_dir("state");
    write_fixture_catalog(&root);
    let _catalog_env = EnvGuard::set("RODER_DISCOVERY_CATALOG_DIR", &root);
    let _state_env = EnvGuard::set("RODER_DISCOVERY_STATE_DIR", &state);

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
    assert!(state.join("discovery/promotions.json").exists());
}

#[tokio::test]
async fn missing_catalog_returns_actionable_fallback_for_all_discovery_tools() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("missing-catalog").join("not-created");
    let _catalog_env = EnvGuard::set("RODER_DISCOVERY_CATALOG_DIR", &root);
    let mut registry = ToolRegistry::default();
    register(&mut registry).unwrap();

    for (name, arguments) in [
        ("discovery.list", json!({})),
        ("discovery.search", json!({ "query": "grep" })),
        ("discovery.read", json!({ "item_id": "tool:builtin/grep" })),
    ] {
        let result = run(
            &registry,
            ToolExecutionContext::new("thread-a", "turn-a", PolicyMode::Default),
            name,
            arguments,
        )
        .await;
        assert!(!result.is_error);
        assert_eq!(result.data["catalogAvailable"], false);
        assert!(result.text.contains("Do not retry"));
        assert!(result.text.contains("already exposed tools"));
        assert!(result.text.contains(root.to_str().unwrap()));
    }
    assert!(!root.exists(), "missing catalogs must not be fabricated");

    write_fixture_catalog(&root);
    let result = run(
        &registry,
        ToolExecutionContext::new("thread-a", "turn-a", PolicyMode::Default),
        "discovery.search",
        json!({ "query": "grep" }),
    )
    .await;
    assert!(result.text.contains("tool:builtin/grep"));
}

#[test]
fn invalid_catalog_is_not_treated_as_unavailable() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("invalid-catalog");
    let _catalog_env = EnvGuard::set("RODER_DISCOVERY_CATALOG_DIR", &root);
    fs::write(root.join("index.json"), "not json").unwrap();
    assert!(load_catalog().is_err());
}

#[test]
fn catalog_read_error_includes_the_catalog_path() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("unreadable-catalog");
    let _catalog_env = EnvGuard::set("RODER_DISCOVERY_CATALOG_DIR", &root);
    fs::create_dir(root.join("index.json")).unwrap();
    let error = load_catalog().unwrap_err();
    assert!(format!("{error:#}").contains(root.to_str().unwrap()));
}

#[test]
fn catalog_paths_follow_config_directory_precedence() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_dir("directory-precedence");
    let config = root.join("config");
    let data = root.join("data");
    let catalog = root.join("catalog-override");
    let state = root.join("state-override");
    let _config_env = EnvGuard::set("RODER_CONFIG_DIR", &config);
    let _data_env = EnvGuard::set("RODER_DATA_DIR", &data);

    assert_eq!(roder_data_dir(), Some(config));
    let _catalog_env = EnvGuard::set("RODER_DISCOVERY_CATALOG_DIR", &catalog);
    let _state_env = EnvGuard::set("RODER_DISCOVERY_STATE_DIR", &state);
    assert_eq!(catalog_root().unwrap(), catalog);
    assert_eq!(promotion_state_dir().unwrap(), state);
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
