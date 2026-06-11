//! Runtime tool-search catalog adapter (roadmap phase 79, Task 2).
//!
//! Combines the per-provider/per-model `ToolSearchConfig` overlay
//! resolution with the provider-safe `ToolSearchCatalog`, so providers and
//! the client-executed search flow build their catalog from exactly the
//! tool set the turn executes with. Execution stays authoritative in
//! Roder: resolved catalog items map back to canonical tool names that
//! flow through `TurnToolExecutor`, permission checks, hooks, and policy
//! mode.

use roder_api::inference::ToolSearchConfig;
use roder_api::tool_search_catalog::ToolSearchCatalog;
use roder_api::tools::ToolSpec;

use crate::RuntimeConfig;

/// Resolves the effective tool-search config for a provider/model and
/// builds the catalog from the turn's tool specs.
pub fn catalog_for_turn(
    cfg: &RuntimeConfig,
    provider: &str,
    model: &str,
    tools: &[ToolSpec],
) -> (ToolSearchConfig, ToolSearchCatalog) {
    let config = crate::runtime::tool_search_for_provider_model(cfg, provider, model);
    let catalog = ToolSearchCatalog::build(tools, &config);
    (config, catalog)
}

#[cfg(test)]
mod tool_search_catalog_tests {
    use super::*;
    use roder_api::inference::{ToolSearchConfigOverlay, ToolSearchMode};

    fn tools() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: "read_file".to_string(),
                description: "Read a file".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
            ToolSpec {
                name: "mcp__jira__search".to_string(),
                description: "Search Jira".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ]
    }

    #[test]
    fn tool_search_catalog_applies_provider_and_model_overlays() {
        let mut cfg = RuntimeConfig::default();
        cfg.provider_tool_search.insert(
            "openai".to_string(),
            ToolSearchConfigOverlay {
                mode: Some(ToolSearchMode::ProviderNative),
                include_mcp: Some(false),
                ..Default::default()
            },
        );

        let (config, catalog) = catalog_for_turn(&cfg, "openai", "gpt-5.5", &tools());
        assert_eq!(config.mode, ToolSearchMode::ProviderNative);
        assert!(!config.include_mcp);
        assert_eq!(catalog.items.len(), 1, "MCP tools filtered by overlay");
        assert_eq!(catalog.items[0].name, "read_file");

        // Other providers keep the base config and the full catalog.
        let (config, catalog) = catalog_for_turn(&cfg, "anthropic", "claude", &tools());
        assert_eq!(config.mode, ToolSearchMode::Explicit);
        assert_eq!(catalog.items.len(), 2);
    }

    #[test]
    fn tool_search_catalog_ids_map_back_to_canonical_tool_names() {
        let cfg = RuntimeConfig::default();
        let (_, catalog) = catalog_for_turn(&cfg, "openai", "gpt-5.5", &tools());
        let hit = catalog.search("read", 1)[0];
        let resolved = catalog.resolve(&hit.id).expect("resolves");
        assert_eq!(resolved.name, "read_file");
    }
}
