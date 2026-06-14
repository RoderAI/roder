//! Provider-safe searchable tool catalog (roadmap phase 79, Task 2).
//!
//! Builds a deterministic, redacted catalog from the tool specs assembled
//! for a turn (registered tools, MCP tools, skill tools, and lazy
//! discovery items are all materialized as `ToolSpec` by the time a turn
//! request is mapped). Catalog payloads are what may leave the process for
//! provider-native tool search and for the client-executed search flow —
//! execution stays authoritative in Roder: a selected catalog item resolves
//! back to the canonical tool name and flows through `TurnToolExecutor`,
//! permission checks, hooks, and policy mode like any other tool call.

use serde::{Deserialize, Serialize};

use crate::inference::ToolSearchConfig;
use crate::tools::ToolSpec;

/// Schemas above this serialized size are dropped from catalog payloads;
/// search ranking only needs names/descriptions, and the full schema is
/// re-attached locally when the tool is selected.
const MAX_SCHEMA_BYTES: usize = 8 * 1024;

/// Catalog source classification, derived from canonical naming
/// conventions (`mcp__server__tool`, `skill:` ids).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCatalogSource {
    Builtin,
    Mcp,
    Skill,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogItem {
    /// Stable catalog id (`tool:<name>`, duplicates suffixed `#2`, `#3`…).
    pub id: String,
    /// Canonical tool name; resolves back to the executable `ToolSpec`.
    pub name: String,
    pub description: String,
    /// Redacted parameter schema; `None` when dropped for size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    pub source: ToolCatalogSource,
}

/// Deterministic, redacted, size-bounded tool catalog for one turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchCatalog {
    pub items: Vec<ToolCatalogItem>,
}

impl ToolSearchCatalog {
    /**
     * Builds the catalog from the turn's tool specs: deterministic
     * name-ordering, stable ids with duplicate-name handling, source
     * filtering per config, `max_catalog_items` limiting, and redaction of
     * credential-like values, internal-only fields, and oversized schemas.
     */
    pub fn build(tools: &[ToolSpec], config: &ToolSearchConfig) -> Self {
        let mut sorted: Vec<&ToolSpec> = tools.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));

        let mut items = Vec::new();
        let mut seen_names: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        for spec in sorted {
            let source = classify_source(&spec.name);
            match source {
                ToolCatalogSource::Mcp if !config.include_mcp => continue,
                ToolCatalogSource::Skill if !config.include_skills => continue,
                _ => {}
            }
            let count = seen_names.entry(spec.name.clone()).or_insert(0);
            *count += 1;
            let id = if *count == 1 {
                format!("tool:{}", spec.name)
            } else {
                format!("tool:{}#{}", spec.name, count)
            };
            items.push(ToolCatalogItem {
                id,
                name: spec.name.clone(),
                description: redact_text(&spec.description),
                parameters: redact_schema(&spec.parameters),
                source,
            });
            if let Some(max) = config.max_catalog_items
                && items.len() >= max as usize
            {
                break;
            }
        }
        Self { items }
    }

    /// Resolves a catalog id (or bare tool name) back to the canonical
    /// tool name for execution through `TurnToolExecutor`.
    pub fn resolve(&self, id_or_name: &str) -> Option<&ToolCatalogItem> {
        self.items
            .iter()
            .find(|item| item.id == id_or_name || item.name == id_or_name)
    }

    /**
     * Local search executor for the client-executed tool-search flow:
     * case-insensitive token matching over names and descriptions, ranked
     * by (name hits, description hits, name) for determinism.
     */
    pub fn search(&self, query: &str, limit: usize) -> Vec<&ToolCatalogItem> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .filter(|token| !token.is_empty())
            .collect();
        if tokens.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, usize, &ToolCatalogItem)> = self
            .items
            .iter()
            .filter_map(|item| {
                let name = item.name.to_ascii_lowercase();
                let description = item.description.to_ascii_lowercase();
                let name_hits = tokens.iter().filter(|token| name.contains(*token)).count();
                let description_hits = tokens
                    .iter()
                    .filter(|token| description.contains(*token))
                    .count();
                (name_hits + description_hits > 0).then_some((name_hits, description_hits, item))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(a.2.name.cmp(&b.2.name))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, item)| item)
            .collect()
    }
}

fn classify_source(name: &str) -> ToolCatalogSource {
    if name.starts_with("mcp__") || name.starts_with("mcp_") {
        ToolCatalogSource::Mcp
    } else if name.starts_with("skill__") || name.starts_with("skill:") {
        ToolCatalogSource::Skill
    } else {
        ToolCatalogSource::Builtin
    }
}

/// Keys whose values must never leave the process in catalog payloads.
fn is_credential_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "token",
        "secret",
        "password",
        "authorization",
        "auth_header",
        "bearer",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Internal-only schema keys stripped from catalog payloads.
fn is_internal_key(key: &str) -> bool {
    key.starts_with("x-roder-") || key == "x-internal" || key.starts_with("x_roder_")
}

fn looks_like_credential(value: &str) -> bool {
    value.starts_with("sk-")
        || value.starts_with("Bearer ")
        || value.starts_with("rk_")
        || value.starts_with("ghp_")
}

/// Strings that leak process-local filesystem layout.
fn looks_like_local_path(value: &str) -> bool {
    value.starts_with("/Users/") || value.starts_with("/home/") || value.starts_with("C:\\Users\\")
}

fn redact_text(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            if looks_like_credential(word) || looks_like_local_path(word) {
                "[redacted]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Redacts a schema for catalog payloads; oversized schemas are dropped.
fn redact_schema(schema: &serde_json::Value) -> Option<serde_json::Value> {
    let redacted = redact_value(schema);
    let size = serde_json::to_vec(&redacted)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    (size <= MAX_SCHEMA_BYTES).then_some(redacted)
}

fn redact_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, entry) in map {
                if is_internal_key(key) {
                    continue;
                }
                if is_credential_key(key) {
                    // Keep the key shape (it is part of the schema) but
                    // never any default/example/const value for it.
                    out.insert(key.clone(), redact_credential_property(entry));
                    continue;
                }
                out.insert(key.clone(), redact_value(entry));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_value).collect())
        }
        serde_json::Value::String(text)
            if looks_like_credential(text) || looks_like_local_path(text) =>
        {
            serde_json::Value::String("[redacted]".to_string())
        }
        other => other.clone(),
    }
}

/// For credential-named schema properties: keep structural keys, drop any
/// value-bearing fields (`default`, `examples`, `const`, `enum`).
fn redact_credential_property(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, entry) in map {
                if matches!(key.as_str(), "default" | "examples" | "const" | "enum") {
                    continue;
                }
                out.insert(key.clone(), redact_value(entry));
            }
            serde_json::Value::Object(out)
        }
        _ => serde_json::Value::String("[redacted]".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, description: &str, parameters: serde_json::Value) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        }
    }

    fn sample_tools() -> Vec<ToolSpec> {
        vec![
            spec(
                "read_file",
                "Read a file from the workspace",
                serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } }
                }),
            ),
            spec(
                "mcp__github__search",
                "Search GitHub issues",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "api_key": { "type": "string", "default": "sk-secret-default" }
                    },
                    "x-roder-internal": { "registry": "/Users/someone/.roder/mcp" }
                }),
            ),
            spec(
                "skill__deploy",
                "Deploy the app per the deploy skill at /Users/me/skills",
                serde_json::json!({}),
            ),
            spec("edit_file", "Edit a file", serde_json::json!({})),
            spec("edit_file", "Duplicate-named tool", serde_json::json!({})),
        ]
    }

    #[test]
    fn tool_search_catalog_is_deterministic_with_stable_ids() {
        let tools = sample_tools();
        let config = ToolSearchConfig::default();
        let first = ToolSearchCatalog::build(&tools, &config);
        let second = ToolSearchCatalog::build(&tools, &config);
        assert_eq!(first, second, "catalogs are stable across runs");

        let ids: Vec<&str> = first.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "tool:edit_file",
                "tool:edit_file#2",
                "tool:mcp__github__search",
                "tool:read_file",
                "tool:skill__deploy",
            ]
        );
        assert_eq!(first.items[2].source, ToolCatalogSource::Mcp);
        assert_eq!(first.items[4].source, ToolCatalogSource::Skill);
    }

    #[test]
    fn tool_search_catalog_redacts_credentials_paths_and_internal_fields() {
        let catalog = ToolSearchCatalog::build(&sample_tools(), &ToolSearchConfig::default());
        let serialized = serde_json::to_string(&catalog).unwrap();
        assert!(!serialized.contains("sk-secret-default"));
        assert!(!serialized.contains("x-roder-internal"));
        assert!(!serialized.contains("/Users/"));
        // The credential-named property's structural shape survives.
        let mcp = catalog.resolve("tool:mcp__github__search").unwrap();
        let parameters = mcp.parameters.as_ref().unwrap();
        assert!(parameters["properties"]["api_key"].get("type").is_some());
        assert!(parameters["properties"]["api_key"].get("default").is_none());
    }

    #[test]
    fn tool_search_catalog_filters_sources_and_limits_items() {
        let tools = sample_tools();
        let config = ToolSearchConfig {
            include_mcp: false,
            include_skills: false,
            ..ToolSearchConfig::default()
        };
        let catalog = ToolSearchCatalog::build(&tools, &config);
        assert!(
            catalog
                .items
                .iter()
                .all(|item| item.source == ToolCatalogSource::Builtin)
        );

        let config = ToolSearchConfig {
            max_catalog_items: Some(2),
            ..ToolSearchConfig::default()
        };
        assert_eq!(ToolSearchCatalog::build(&tools, &config).items.len(), 2);
    }

    #[test]
    fn tool_search_catalog_search_ranks_and_resolves_to_canonical_specs() {
        let catalog = ToolSearchCatalog::build(&sample_tools(), &ToolSearchConfig::default());
        let results = catalog.search("read file", 3);
        assert_eq!(results[0].name, "read_file", "name hits rank first");
        assert!(catalog.search("", 5).is_empty());
        assert!(catalog.search("zzz-nothing", 5).is_empty());

        // Selected ids resolve back to canonical tool names for execution.
        let resolved = catalog.resolve(&results[0].id).unwrap();
        assert_eq!(resolved.name, "read_file");
        assert_eq!(catalog.resolve("edit_file").unwrap().id, "tool:edit_file");

        let oversized = spec(
            "big",
            "Tool with oversized schema",
            serde_json::json!({ "blob": "x".repeat(20_000) }),
        );
        let catalog = ToolSearchCatalog::build(
            std::slice::from_ref(&oversized),
            &ToolSearchConfig::default(),
        );
        assert!(
            catalog.items[0].parameters.is_none(),
            "oversized schemas are dropped"
        );
    }
}
