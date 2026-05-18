use std::sync::Arc;

use roder_api::memory::{MemoryQuery, MemoryRecord, MemoryScope, MemoryStore};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;

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
            name: "memory.save",
        }))?;
        registry.register(Arc::new(MemorySaveTool {
            store: self.store.clone(),
            name: "save_memory",
        }))?;
        registry.register(Arc::new(MemoryQueryTool {
            store: self.store.clone(),
            name: "memory.query",
        }))?;
        registry.register(Arc::new(MemoryQueryTool {
            store: self.store.clone(),
            name: "query_memories",
        }))?;
        registry.register(Arc::new(MemoryReadTool {
            store: self.store.clone(),
            name: "memory.read",
        }))?;
        registry.register(Arc::new(MemoryReadTool {
            store: self.store.clone(),
            name: "read_memory",
        }))?;
        registry.register(Arc::new(MemoryDeleteTool {
            store: self.store.clone(),
            name: "memory.delete",
        }))?;
        registry.register(Arc::new(MemoryDeleteTool {
            store: self.store.clone(),
            name: "delete_memory",
        }))?;
        registry.register(Arc::new(MemoryUpdateTool {
            store: self.store.clone(),
            name: "memory.update",
        }))?;
        registry.register(Arc::new(MemoryUpdateTool {
            store: self.store.clone(),
            name: "update_memory",
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
    #[serde(default)]
    include_global: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct IdArgs {
    id: String,
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
        Ok(result(
            call,
            format!("{} memories", results.len()),
            json!({ "results": results }),
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
        Ok(result(
            call,
            record.as_ref().map(|r| r.text.clone()).unwrap_or_default(),
            json!({ "memory": record }),
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
            "includeGlobal": { "type": "boolean" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
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
            "properties": { "id": { "type": "string" } },
            "required": ["id"],
            "additionalProperties": false
        }),
    }
}
