use std::sync::Arc;

use roder_api::memory::{MemoryQuery, MemoryRecord, MemoryScope, MemoryStore};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;

use crate::response_format::{ResponseFormat, render_memory_query_results};

/// Cap on memory text accepted by save/update. Stored notes are re-injected
/// into future turns by the memory context provider, so unbounded text would
/// flood prompts; the model is told to store a shorter note instead.
const MAX_MEMORY_TEXT_BYTES: usize = 16 * 1024;

fn ensure_text_within_limit(text: &str) -> anyhow::Result<()> {
    if text.len() > MAX_MEMORY_TEXT_BYTES {
        anyhow::bail!(
            "memory text is {} bytes; the limit is {MAX_MEMORY_TEXT_BYTES} bytes — store a shorter, more focused note",
            text.len()
        );
    }
    Ok(())
}

pub struct MemoryToolContributor {
    store: Arc<dyn MemoryStore>,
}

impl MemoryToolContributor {
    pub fn new(store: Arc<dyn MemoryStore>) -> Self {
        Self { store }
    }
}

impl roder_api::tools::ToolContributor for MemoryToolContributor {
    fn id(&self) -> roder_api::extension::ToolProviderId {
        "memory-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(MemorySaveTool {
            store: self.store.clone(),
            name: "memory_save",
        }))?;
        registry.register(Arc::new(MemoryQueryTool {
            store: self.store.clone(),
            name: "memory_query",
        }))?;
        registry.register(Arc::new(MemoryReadTool {
            store: self.store.clone(),
            name: "memory_read",
        }))?;
        registry.register(Arc::new(MemoryDeleteTool {
            store: self.store.clone(),
            name: "memory_delete",
        }))?;
        registry.register(Arc::new(MemoryUpdateTool {
            store: self.store.clone(),
            name: "memory_update",
        }))
    }
}

#[derive(Debug, Deserialize)]
struct SaveArgs {
    text: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct QueryArgs {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    /// The schema briefly advertised `includeGlobal`; the alias keeps
    /// models that learned the camelCase spelling working.
    #[serde(default, alias = "includeGlobal")]
    include_global: bool,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    response_format: ResponseFormat,
}

#[derive(Debug, Deserialize)]
struct IdArgs {
    id: String,
    #[serde(default)]
    response_format: ResponseFormat,
}

#[derive(Debug, Deserialize)]
struct UpdateArgs {
    id: String,
    text: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

struct MemorySaveTool {
    store: Arc<dyn MemoryStore>,
    name: &'static str,
}

struct MemoryQueryTool {
    store: Arc<dyn MemoryStore>,
    name: &'static str,
}

struct MemoryReadTool {
    store: Arc<dyn MemoryStore>,
    name: &'static str,
}

struct MemoryDeleteTool {
    store: Arc<dyn MemoryStore>,
    name: &'static str,
}

struct MemoryUpdateTool {
    store: Arc<dyn MemoryStore>,
    name: &'static str,
}

#[async_trait::async_trait]
impl ToolExecutor for MemorySaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: "Saves a project or global memory.".to_string(),
            parameters: save_schema(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<SaveArgs>(&call)?;
        ensure_text_within_limit(&args.text)?;
        let now = OffsetDateTime::now_utc();
        let record = MemoryRecord {
            id: None,
            scope: parse_scope(args.scope.as_deref()),
            text: args.text,
            content_hash: None,
            metadata: args.metadata,
            usage: None,
            deleted: false,
            created_at: now,
            updated_at: now,
        };
        let id = self.store.put(record).await?;
        Ok(result(
            call,
            format!("saved memory {id}"),
            json!({ "id": id }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MemoryQueryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: "Queries local memories.".to_string(),
            parameters: query_schema(),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<QueryArgs>(&call)?;
        let results = self
            .store
            .search(MemoryQuery {
                scope: args.scope.as_deref().map(|scope| parse_scope(Some(scope))),
                text: args.query,
                limit: args.limit.unwrap_or(10),
                include_global: args.include_global,
                provider_id: None,
                model: None,
            })
            .await?;
        let text = render_memory_query_results(&results, args.response_format);
        Ok(result(
            call,
            text,
            json!({
                "response_format": args.response_format.as_str(),
                "results": results
            }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MemoryReadTool {
    fn spec(&self) -> ToolSpec {
        id_spec(self.name, "Reads one memory by id.")
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<IdArgs>(&call)?;
        let record = self.store.get(&args.id).await?;
        let text = record
            .as_ref()
            .map(|r| args.response_format.format_memory_text(&r.text))
            .unwrap_or_default();
        Ok(result(
            call,
            text,
            json!({
                "response_format": args.response_format.as_str(),
                "memory": record
            }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MemoryDeleteTool {
    fn spec(&self) -> ToolSpec {
        id_spec(self.name, "Deletes one memory by id.")
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<IdArgs>(&call)?;
        self.store.delete(&args.id).await?;
        Ok(result(
            call,
            format!("deleted memory {}", args.id),
            json!({ "deleted": true }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for MemoryUpdateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.to_string(),
            description: "Updates one memory by id.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "text": { "type": "string" },
                    "metadata": { "type": "object" }
                },
                "required": ["id", "text"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<UpdateArgs>(&call)?;
        ensure_text_within_limit(&args.text)?;
        let existing = self
            .store
            .get(&args.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("memory not found: {}", args.id))?;
        let now = OffsetDateTime::now_utc();
        let record = MemoryRecord {
            id: Some(args.id.clone()),
            scope: existing.scope,
            text: args.text,
            content_hash: None,
            metadata: args.metadata,
            usage: existing.usage,
            deleted: false,
            created_at: existing.created_at,
            updated_at: now,
        };
        self.store.put(record).await?;
        Ok(result(
            call,
            format!("updated memory {}", args.id),
            json!({ "id": args.id }),
        ))
    }
}

fn parse<T: serde::de::DeserializeOwned>(call: &ToolCall) -> anyhow::Result<T> {
    Ok(serde_json::from_value(call.arguments.clone())?)
}

fn result(call: ToolCall, text: String, data: serde_json::Value) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text,
        data,
        is_error: false,
    }
}

fn parse_scope(scope: Option<&str>) -> MemoryScope {
    match scope.unwrap_or("project") {
        "global" => MemoryScope::Global,
        value if value.starts_with("project:") => {
            MemoryScope::Project(value.trim_start_matches("project:").to_string())
        }
        value if value.starts_with("workspace:") => {
            MemoryScope::Workspace(value.trim_start_matches("workspace:").to_string())
        }
        value => MemoryScope::Project(value.to_string()),
    }
}

fn save_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "text": { "type": "string" },
            "scope": { "type": "string", "description": "global, project, or project:<id>" },
            "metadata": { "type": "object" }
        },
        "required": ["text"],
        "additionalProperties": false
    })
}

fn query_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string" },
            "scope": { "type": "string" },
            "include_global": { "type": "boolean" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50 },
            "response_format": response_format_schema()
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn id_spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "response_format": response_format_schema()
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    }
}

fn response_format_schema() -> serde_json::Value {
    json!({
        "type": "string",
        "enum": ["concise", "detailed"],
        "default": "concise",
        "description": "concise keeps model-facing memory text bounded; detailed returns full memory text."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteMemoryStore;
    use roder_api::memory::MemoryRecord;
    use roder_api::tools::ToolExecutionContext;
    use std::sync::Arc;

    #[tokio::test]
    async fn memory_query_response_format_includes_ids_and_bounds_text() {
        let store = Arc::new(test_store());
        let id = seed_memory(store.as_ref(), "sqlite memory ".repeat(80)).await;
        let tool = MemoryQueryTool {
            store,
            name: "memory_query",
        };

        let concise = tool
            .execute(
                context(),
                call("memory_query", json!({"query": "sqlite memory"})),
            )
            .await
            .unwrap();
        let detailed = tool
            .execute(
                context(),
                call(
                    "memory_query",
                    json!({"query": "sqlite memory", "response_format": "detailed"}),
                ),
            )
            .await
            .unwrap();

        assert_eq!(concise.data["response_format"], "concise");
        assert!(concise.text.contains(&id));
        assert!(concise.text.contains("..."));
        assert!(concise.text.len() < detailed.text.len());
        assert_eq!(detailed.data["response_format"], "detailed");
    }

    #[tokio::test]
    async fn memory_read_response_format_controls_text() {
        let store = Arc::new(test_store());
        let id = seed_memory(store.as_ref(), "read memory ".repeat(80)).await;
        let tool = MemoryReadTool {
            store,
            name: "memory_read",
        };

        let concise = tool
            .execute(context(), call("memory_read", json!({"id": id})))
            .await
            .unwrap();
        let detailed = tool
            .execute(
                context(),
                call(
                    "memory_read",
                    json!({"id": id, "response_format": "detailed"}),
                ),
            )
            .await
            .unwrap();

        assert_eq!(concise.data["response_format"], "concise");
        assert!(concise.text.contains("..."));
        assert!(concise.text.len() < detailed.text.len());
        assert_eq!(detailed.data["response_format"], "detailed");
    }

    #[tokio::test]
    async fn include_global_accepts_both_schema_and_legacy_camel_case_spelling() {
        let store = Arc::new(test_store());
        seed_global_memory(store.as_ref(), "the global launch codeword is heron").await;
        let tool = MemoryQueryTool {
            store,
            name: "memory_query",
        };

        for (label, arguments) in [
            (
                "include_global",
                json!({"query": "launch codeword", "scope": "project:p", "include_global": true}),
            ),
            (
                "includeGlobal",
                json!({"query": "launch codeword", "scope": "project:p", "includeGlobal": true}),
            ),
        ] {
            let result = tool
                .execute(context(), call("memory_query", arguments))
                .await
                .unwrap();
            assert!(
                result.text.contains("heron"),
                "{label}: scoped query did not fold in the global record: {}",
                result.text
            );
        }
    }

    #[tokio::test]
    async fn memory_save_rejects_oversized_text() {
        let store = Arc::new(test_store());
        let tool = MemorySaveTool {
            store,
            name: "memory_save",
        };

        let error = tool
            .execute(
                context(),
                call(
                    "memory_save",
                    json!({"text": "x".repeat(MAX_MEMORY_TEXT_BYTES + 1)}),
                ),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("store a shorter"));
    }

    #[tokio::test]
    async fn memory_update_rejects_oversized_text() {
        let store = Arc::new(test_store());
        let id = seed_memory(store.as_ref(), "small note".to_string()).await;
        let tool = MemoryUpdateTool {
            store,
            name: "memory_update",
        };

        let error = tool
            .execute(
                context(),
                call(
                    "memory_update",
                    json!({"id": id, "text": "x".repeat(MAX_MEMORY_TEXT_BYTES + 1)}),
                ),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("store a shorter"));
    }

    async fn seed_global_memory(store: &SqliteMemoryStore, text: &str) {
        let now = OffsetDateTime::now_utc();
        store
            .put(MemoryRecord {
                id: None,
                scope: MemoryScope::Global,
                text: text.to_string(),
                content_hash: None,
                metadata: json!({}),
                usage: None,
                deleted: false,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }

    async fn seed_memory(store: &SqliteMemoryStore, text: String) -> String {
        let now = OffsetDateTime::now_utc();
        store
            .put(MemoryRecord {
                id: None,
                scope: MemoryScope::Project("p".to_string()),
                text,
                content_hash: None,
                metadata: json!({}),
                usage: None,
                deleted: false,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap()
    }

    fn test_store() -> SqliteMemoryStore {
        let path =
            std::env::temp_dir().join(format!("roder-memory-{}.sqlite3", uuid::Uuid::new_v4()));
        SqliteMemoryStore::open(path).unwrap()
    }

    fn context() -> ToolExecutionContext {
        ToolExecutionContext::new(
            "thread-a",
            "turn-a",
            roder_api::policy_mode::PolicyMode::Default,
        )
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: arguments.to_string(),
            arguments,
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
        }
    }
}
