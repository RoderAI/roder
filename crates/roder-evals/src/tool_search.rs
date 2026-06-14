//! Offline provider-native tool-search eval harness (roadmap phase 79).
//!
//! Fixtures under `evals/tool_search/` describe a searchable tool catalog,
//! the requested tool-search mode, and a scripted provider search/selection
//! outcome. The harness builds the provider-safe catalog, resolves the
//! effective mode through the canonical `ToolSearchConfig` contract, maps the
//! request body through the real OpenAI Responses / Anthropic Messages
//! mappers, and simulates searched-tool selection so error cases (unknown
//! tool ids, malformed results, denied permissions, redaction) fail closed
//! with actionable diagnostics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use roder_api::inference::{
    AgentInferenceRequest, EffectiveToolSearchMode, InstructionBundle, ModelSelection,
    OutputConfig, ReasoningConfig, RuntimeHints, ToolSearchConfig, ToolSearchMode,
    ToolSearchProviderVariant,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchEvalFixture {
    pub id: String,
    pub title: String,
    /// `openai` or `anthropic`.
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub mode: ToolSearchMode,
    #[serde(default = "default_true")]
    pub fallback_to_explicit_tools: bool,
    #[serde(default)]
    pub provider_variant: ToolSearchProviderVariant,
    #[serde(default)]
    pub catalog: ToolSearchCatalogFixture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<ToolSearchScript>,
    pub expected: ToolSearchExpectation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchCatalogFixture {
    #[serde(default)]
    pub tools: Vec<ToolSearchCatalogTool>,
    /// Synthesizes `count` extra tools so large-catalog fixtures stay small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<GeneratedCatalogFixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchCatalogTool {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /**
     * Internal-only metadata (credentials, auth headers, local paths) that
     * the provider-safe catalog must never forward. Values listed here must
     * not appear anywhere in the serialized provider request body.
     */
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub internal_metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedCatalogFixture {
    pub count: u32,
    pub name_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchScript {
    pub query: String,
    /// Raw provider search-results payload; may be intentionally malformed.
    pub results: Value,
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchExpectation {
    pub outcome: ToolSearchExpectedOutcome,
    #[serde(default)]
    pub diagnostic_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_tools: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_tool_search_entry: Option<bool>,
    #[serde(default)]
    pub executed_tools: Vec<String>,
    #[serde(default)]
    pub body_must_not_contain: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchExpectedOutcome {
    /// The request mapped (explicit or provider-native) without selection.
    #[default]
    RequestMapped,
    /// Scripted search selection resolved and executed permitted tools.
    Executed,
    /// The turn failed closed with an actionable diagnostic.
    FailClosed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSearchOutcome {
    RequestMapped {
        effective_mode: EffectiveToolSearchMode,
        body: Value,
        catalog_items: usize,
        deferred_tools: usize,
        native_tool_search_entry: bool,
    },
    Executed {
        executed_tools: Vec<String>,
        body: Value,
    },
    FailClosed {
        diagnostic: String,
    },
}

impl ToolSearchOutcome {
    pub fn diagnostic(&self) -> Option<&str> {
        match self {
            Self::FailClosed { diagnostic } => Some(diagnostic),
            _ => None,
        }
    }
}

pub fn load_tool_search_fixtures(dir: &Path) -> anyhow::Result<Vec<ToolSearchEvalFixture>> {
    let mut fixtures = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        // The catalog-adapter snapshot has its own schema and test.
        if path.file_name().and_then(|name| name.to_str()) == Some("catalog_fixture.json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let fixture: ToolSearchEvalFixture = serde_json::from_str(&text)
            .map_err(|err| anyhow::anyhow!("{}: {err}", path.display()))?;
        fixtures.push(fixture);
    }
    fixtures.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(fixtures)
}

pub fn default_tool_search_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals/tool_search")
}

/// Catalog-adapter snapshot fixture (phase 79 Task 2): a fixed toolset with
/// the expected stable catalog ids, source classification, redaction
/// needles, and a ranked search expectation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogAdapterFixture {
    pub id: String,
    pub tools: Vec<ToolSpec>,
    pub expected: CatalogAdapterExpectation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogAdapterExpectation {
    pub ids: Vec<String>,
    pub sources: Vec<String>,
    pub forbidden_needles: Vec<String>,
    pub search_query: String,
    pub search_top_hit: String,
}

pub fn load_catalog_adapter_fixture() -> anyhow::Result<CatalogAdapterFixture> {
    let path = default_tool_search_fixture_dir().join("catalog_fixture.json");
    let text = std::fs::read_to_string(&path)?;
    serde_json::from_str(&text).map_err(|err| anyhow::anyhow!("{}: {err}", path.display()))
}

/**
 * Build the provider-safe searchable catalog: deterministic name ordering,
 * duplicate-name removal, max-item limiting, and redaction of internal-only
 * metadata. Only name/description/parameters ever reach the provider.
 */
pub fn build_provider_safe_catalog(catalog: &ToolSearchCatalogFixture) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSearchCatalogTool> = catalog.tools.clone();
    if let Some(generated) = &catalog.generated {
        for index in 0..generated.count {
            tools.push(ToolSearchCatalogTool {
                name: format!("{}_{index:04}", generated.name_prefix),
                description: format!(
                    "Generated catalog tool {index} for {}",
                    generated.name_prefix
                ),
                parameters: None,
                internal_metadata: BTreeMap::new(),
            });
        }
    }
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools.dedup_by(|left, right| left.name == right.name);
    if let Some(max_items) = catalog.max_items {
        tools.truncate(max_items as usize);
    }
    tools
        .into_iter()
        .map(|tool| ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool
                .parameters
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        })
        .collect()
}

pub fn run_tool_search_fixture(
    fixture: &ToolSearchEvalFixture,
) -> anyhow::Result<ToolSearchOutcome> {
    let catalog = build_provider_safe_catalog(&fixture.catalog);
    let config = ToolSearchConfig {
        mode: fixture.mode,
        max_catalog_items: fixture.catalog.max_items,
        fallback_to_explicit_tools: fixture.fallback_to_explicit_tools,
        provider_variant: fixture.provider_variant,
        ..ToolSearchConfig::default()
    };
    let provider_native_supported = match fixture.provider.as_str() {
        "openai" => roder_ext_openai_responses::openai_model_supports_tool_search(&fixture.model),
        "anthropic" => roder_ext_anthropic::anthropic_model_supports_tool_search(&fixture.model),
        other => anyhow::bail!("unsupported fixture provider: {other}"),
    };
    let effective_mode = match config.resolve_effective_mode(provider_native_supported) {
        Ok(mode) => mode,
        Err(error) => {
            return Ok(ToolSearchOutcome::FailClosed {
                diagnostic: error.to_string(),
            });
        }
    };

    let request = inference_request(fixture, &catalog, &config);
    let body = match fixture.provider.as_str() {
        "openai" => roder_ext_openai_responses::OpenAiResponsesEngine::map_request(&request),
        "anthropic" => roder_ext_anthropic::AnthropicEngine::map_request(&request),
        other => anyhow::bail!("unsupported fixture provider: {other}"),
    };
    let (deferred_tools, native_tool_search_entry) = body_tool_search_shape(&body);

    let Some(search) = &fixture.search else {
        return Ok(ToolSearchOutcome::RequestMapped {
            effective_mode,
            body,
            catalog_items: catalog.len(),
            deferred_tools,
            native_tool_search_entry,
        });
    };

    match resolve_search_selection(search, &catalog) {
        Ok(executed_tools) => Ok(ToolSearchOutcome::Executed {
            executed_tools,
            body,
        }),
        Err(diagnostic) => Ok(ToolSearchOutcome::FailClosed { diagnostic }),
    }
}

/**
 * Resolve a scripted provider search-result payload into executed catalog
 * tools, failing closed for malformed payloads, unknown selected tool ids,
 * and permission-denied selections.
 */
fn resolve_search_selection(
    search: &ToolSearchScript,
    catalog: &[ToolSpec],
) -> Result<Vec<String>, String> {
    let Some(results) = search.results.as_array() else {
        return Err(format!(
            "malformed provider tool-search results for query {:?}: expected an array of \
             {{\"name\": ...}} objects, got {}; failing closed without executing any tool",
            search.query, search.results
        ));
    };
    let mut executed = Vec::new();
    for result in results {
        let Some(name) = result.get("name").and_then(Value::as_str) else {
            return Err(format!(
                "malformed provider tool-search result entry {result} for query {:?}: missing \
                 string \"name\"; failing closed without executing any tool",
                search.query
            ));
        };
        if !catalog.iter().any(|tool| tool.name == name) {
            return Err(format!(
                "provider selected unknown tool id {name:?} that is not in the provider-safe \
                 catalog; failing closed without executing any tool"
            ));
        }
        if search.denied_tools.iter().any(|denied| denied == name) {
            return Err(format!(
                "permission denied for searched tool {name:?}; provider-native tool search does \
                 not bypass Roder permission checks"
            ));
        }
        executed.push(name.to_string());
    }
    Ok(executed)
}

fn inference_request(
    fixture: &ToolSearchEvalFixture,
    catalog: &[ToolSpec],
    config: &ToolSearchConfig,
) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: fixture.provider.clone(),
            model: fixture.model.clone(),
        },
        instructions: InstructionBundle {
            system: Some("offline tool-search eval".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![roder_api::transcript::TranscriptItem::UserMessage(
            roder_api::transcript::UserMessage::text("run the fixture task"),
        )],
        tools: catalog.to_vec(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        output: OutputConfig {
            max_tokens: Some(512),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints {
            tool_search: config.clone(),
            ..RuntimeHints::default()
        },
        metadata: json!({}),
    }
}

/// Count deferred tool entries and detect the provider-native search entry in
/// a mapped OpenAI Responses or Anthropic Messages request body.
fn body_tool_search_shape(body: &Value) -> (usize, bool) {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return (0, false);
    };
    let deferred = tools
        .iter()
        .filter(|tool| tool.get("defer_loading").and_then(Value::as_bool) == Some(true))
        .count();
    let native_entry = tools.iter().any(|tool| {
        tool.get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind == "tool_search" || kind.starts_with("tool_search_tool_"))
    });
    (deferred, native_entry)
}

pub fn assert_tool_search_fixture(fixture: &ToolSearchEvalFixture) -> anyhow::Result<()> {
    let outcome = run_tool_search_fixture(fixture)?;
    let expected = &fixture.expected;
    let body = match &outcome {
        ToolSearchOutcome::RequestMapped { body, .. }
        | ToolSearchOutcome::Executed { body, .. } => Some(body.clone()),
        ToolSearchOutcome::FailClosed { .. } => None,
    };

    match (expected.outcome, &outcome) {
        (
            ToolSearchExpectedOutcome::RequestMapped,
            ToolSearchOutcome::RequestMapped {
                catalog_items,
                deferred_tools,
                native_tool_search_entry,
                ..
            },
        ) => {
            if let Some(expected_items) = expected.catalog_items {
                anyhow::ensure!(
                    *catalog_items == expected_items,
                    "{}: expected {expected_items} catalog items, got {catalog_items}",
                    fixture.id
                );
            }
            if let Some(expected_deferred) = expected.deferred_tools {
                anyhow::ensure!(
                    *deferred_tools == expected_deferred,
                    "{}: expected {expected_deferred} deferred tools, got {deferred_tools}",
                    fixture.id
                );
            }
            if let Some(expected_entry) = expected.native_tool_search_entry {
                anyhow::ensure!(
                    *native_tool_search_entry == expected_entry,
                    "{}: expected native tool-search entry = {expected_entry}",
                    fixture.id
                );
            }
        }
        (
            ToolSearchExpectedOutcome::Executed,
            ToolSearchOutcome::Executed { executed_tools, .. },
        ) => {
            anyhow::ensure!(
                *executed_tools == expected.executed_tools,
                "{}: expected executed tools {:?}, got {executed_tools:?}",
                fixture.id,
                expected.executed_tools
            );
        }
        (ToolSearchExpectedOutcome::FailClosed, ToolSearchOutcome::FailClosed { diagnostic }) => {
            for needle in &expected.diagnostic_contains {
                anyhow::ensure!(
                    diagnostic.contains(needle),
                    "{}: diagnostic {diagnostic:?} missing {needle:?}",
                    fixture.id
                );
            }
        }
        (expected_outcome, actual) => anyhow::bail!(
            "{}: expected outcome {expected_outcome:?}, got {actual:?}",
            fixture.id
        ),
    }

    if let Some(body) = body {
        let serialized = body.to_string();
        for marker in &expected.body_must_not_contain {
            anyhow::ensure!(
                !serialized.contains(marker),
                "{}: provider request body leaked redacted marker {marker:?}",
                fixture.id
            );
        }
        for tool in fixture
            .catalog
            .tools
            .iter()
            .flat_map(|tool| tool.internal_metadata.values())
        {
            anyhow::ensure!(
                !serialized.contains(tool.as_str()),
                "{}: provider request body leaked internal metadata value {tool:?}",
                fixture.id
            );
        }
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
#[path = "tool_search_tests.rs"]
mod tool_search_tests;
