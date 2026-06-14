//! `knowledge_*` tools: the agent's CRUD/search/link surface over the
//! project knowledge base.

use std::sync::Arc;

use roder_api::knowledge::{
    KnowledgeKind, KnowledgeLinkRequest, KnowledgeLinkType, KnowledgeListQuery, KnowledgeQuery,
    KnowledgeSaveRequest, KnowledgeSource, KnowledgeStatus, KnowledgeStore, KnowledgeUpdateRequest,
};
use roder_api::memory::MemoryScope;
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

/// Cap on lines returned by a single `knowledge_read` page so long documents
/// stay paginated instead of flooding the model context.
const READ_PAGE_LINES: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 25;
const DEFAULT_SEARCH_LIMIT: usize = 8;

pub struct KnowledgeToolContributor {
    store: Arc<dyn KnowledgeStore>,
}

impl KnowledgeToolContributor {
    pub fn new(store: Arc<dyn KnowledgeStore>) -> Self {
        Self { store }
    }
}

impl roder_api::tools::ToolContributor for KnowledgeToolContributor {
    fn id(&self) -> roder_api::extension::ToolProviderId {
        "knowledge-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        registry.register(Arc::new(KnowledgeListTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeReadTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeSearchTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeSaveTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeUpdateTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeDeleteTool {
            store: self.store.clone(),
        }))?;
        registry.register(Arc::new(KnowledgeLinkTool {
            store: self.store.clone(),
        }))
    }
}

pub fn parse_scope(scope: Option<&str>) -> MemoryScope {
    match scope.unwrap_or("project") {
        "global" => MemoryScope::Global,
        "project" => MemoryScope::Project(default_project_key()),
        value if value.starts_with("project:") => {
            MemoryScope::Project(value.trim_start_matches("project:").to_string())
        }
        value => MemoryScope::Project(value.to_string()),
    }
}

/// Project key fallback used when no explicit project id is given: the
/// current directory name, matching the memory CLI's project resolution.
pub fn default_project_key() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "default".to_string())
}

fn parse_link_type(value: &str) -> anyhow::Result<KnowledgeLinkType> {
    match value {
        "relates_to" => Ok(KnowledgeLinkType::RelatesTo),
        "supersedes" => Ok(KnowledgeLinkType::Supersedes),
        "derived_from" => Ok(KnowledgeLinkType::DerivedFrom),
        "contradicts" => Ok(KnowledgeLinkType::Contradicts),
        "duplicates" => Ok(KnowledgeLinkType::Duplicates),
        other => anyhow::bail!(
            "unknown link type {other:?}; expected relates_to, supersedes, derived_from, contradicts, or duplicates"
        ),
    }
}

fn parse_status(value: &str) -> anyhow::Result<KnowledgeStatus> {
    match value {
        "active" => Ok(KnowledgeStatus::Active),
        "draft" => Ok(KnowledgeStatus::Draft),
        "superseded" => Ok(KnowledgeStatus::Superseded),
        "archived" => Ok(KnowledgeStatus::Archived),
        other => anyhow::bail!(
            "unknown status {other:?}; expected active, draft, superseded, or archived"
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ListArgs {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    id: String,
    #[serde(default)]
    revision: Option<u32>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default, alias = "includeGlobal")]
    include_global: bool,
}

#[derive(Debug, Deserialize)]
struct SaveArgs {
    kind: String,
    title: String,
    body: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateArgs {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct IdArgs {
    id: String,
}

#[derive(Debug, Deserialize)]
struct LinkArgs {
    from: String,
    to: String,
    #[serde(rename = "type")]
    link_type: String,
    #[serde(default)]
    remove: bool,
}

struct KnowledgeListTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeReadTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeSearchTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeSaveTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeUpdateTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeDeleteTool {
    store: Arc<dyn KnowledgeStore>,
}

struct KnowledgeLinkTool {
    store: Arc<dyn KnowledgeStore>,
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_list".to_string(),
            description:
                "Lists project knowledge documents (requirements, decisions, research, runbooks, notes). Filter by kind, tag, or status."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "description": "global, project, or project:<id>" },
                    "kind": { "type": "string", "description": "memory, requirement, decision, research, runbook, artifact, note, or a custom kind" },
                    "tag": { "type": "string" },
                    "status": { "type": "string", "enum": ["active", "draft", "superseded", "archived"] },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ListArgs>(&call)?;
        let summaries = self
            .store
            .list(KnowledgeListQuery {
                scope: Some(parse_scope(args.scope.as_deref())),
                kind: args.kind.as_deref().map(KnowledgeKind::parse),
                tag: args.tag,
                status: args.status.as_deref().map(parse_status).transpose()?,
                include_archived: false,
                limit: args.limit.unwrap_or(DEFAULT_LIST_LIMIT).min(100),
            })
            .await?;
        let text = if summaries.is_empty() {
            "no knowledge documents found".to_string()
        } else {
            summaries
                .iter()
                .map(|doc| {
                    format!(
                        "{}\t{}\t{}\t[{}] rev {}{}",
                        doc.id,
                        doc.kind,
                        doc.title,
                        doc.status.as_str(),
                        doc.revision,
                        if doc.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" tags: {}", doc.tags.join(","))
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(result(call, text, json!({ "documents": summaries })))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_read".to_string(),
            description: "Reads a knowledge document by id, paginated by line offset. Optionally reads a specific prior revision.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "revision": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0, "description": "Line offset for pagination." }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<ReadArgs>(&call)?;
        let doc = match args.revision {
            Some(revision) => self.store.get_revision(&args.id, revision).await?,
            None => self.store.get(&args.id).await?,
        };
        let Some(doc) = doc else {
            return Ok(result(
                call,
                format!("knowledge document not found: {}", args.id),
                json!({ "document": null }),
            ));
        };
        let offset = args.offset.unwrap_or(0);
        let lines = doc.body.lines().collect::<Vec<_>>();
        let total = lines.len();
        let page = lines
            .iter()
            .skip(offset)
            .take(READ_PAGE_LINES)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let shown_end = (offset + READ_PAGE_LINES).min(total);
        let mut text = format!(
            "# {} ({}, {}, rev {})\n\n{}",
            doc.title,
            doc.kind,
            doc.status.as_str(),
            doc.revision,
            page
        );
        if shown_end < total {
            text.push_str(&format!(
                "\n\n[lines {}-{} of {}; call knowledge_read with offset={} for more]",
                offset + 1,
                shown_end,
                total,
                shown_end
            ));
        }
        Ok(result(
            call,
            text,
            json!({
                "document": doc.summary(),
                "links": doc.links,
                "totalLines": total,
                "offset": offset,
            }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_search".to_string(),
            description: "Searches project knowledge documents and returns scored snippets with document ids.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "scope": { "type": "string" },
                    "kind": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 25 },
                    "include_global": { "type": "boolean" }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<SearchArgs>(&call)?;
        let results = self
            .store
            .search(KnowledgeQuery {
                scope: Some(parse_scope(args.scope.as_deref())),
                text: args.query,
                kind: args.kind.as_deref().map(KnowledgeKind::parse),
                limit: args.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(25),
                include_global: args.include_global,
            })
            .await?;
        let text = if results.is_empty() {
            "no matching knowledge documents".to_string()
        } else {
            results
                .iter()
                .map(|result| {
                    format!(
                        "{:.3}\t{}\t{}\t{}\n\t{}",
                        result.score,
                        result.document.id,
                        result.document.kind,
                        result.document.title,
                        result.snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(result(call, text, json!({ "results": results })))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeSaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_save".to_string(),
            description: "Saves a new knowledge document (requirement, decision, research, runbook, memory narrative, artifact reference, or note).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "memory, requirement, decision, research, runbook, artifact, note, or a custom kind" },
                    "title": { "type": "string" },
                    "body": { "type": "string", "description": "Markdown body." },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "scope": { "type": "string", "description": "global, project, or project:<id>" }
                },
                "required": ["kind", "title", "body"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<SaveArgs>(&call)?;
        let doc = self
            .store
            .save(KnowledgeSaveRequest {
                scope: parse_scope(args.scope.as_deref()),
                kind: KnowledgeKind::parse(&args.kind),
                title: args.title,
                tags: args.tags,
                body: args.body,
                source: KnowledgeSource::Agent,
            })
            .await?;
        Ok(result(
            call,
            format!("saved knowledge document {} ({})", doc.id, doc.slug),
            json!({ "document": doc.summary() }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeUpdateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_update".to_string(),
            description: "Updates a knowledge document; absent fields keep their current value. Writes a new revision.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                    "status": { "type": "string", "enum": ["active", "draft", "superseded", "archived"] },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id"],
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
        let doc = self
            .store
            .update(KnowledgeUpdateRequest {
                id: args.id,
                title: args.title,
                body: args.body,
                status: args.status.as_deref().map(parse_status).transpose()?,
                tags: args.tags,
                source: KnowledgeSource::Agent,
            })
            .await?;
        Ok(result(
            call,
            format!(
                "updated knowledge document {} to revision {}",
                doc.id, doc.revision
            ),
            json!({ "document": doc.summary() }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeDeleteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_delete".to_string(),
            description: "Archives a knowledge document. Archived documents leave lists and search but stay readable by id.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<IdArgs>(&call)?;
        let archived = self.store.archive(&args.id).await?;
        Ok(result(
            call,
            if archived {
                format!("archived knowledge document {}", args.id)
            } else {
                format!("knowledge document {} was not active", args.id)
            },
            json!({ "archived": archived }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for KnowledgeLinkTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "knowledge_link".to_string(),
            description: "Adds or removes a typed link between two knowledge documents."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "type": { "type": "string", "enum": ["relates_to", "supersedes", "derived_from", "contradicts", "duplicates"] },
                    "remove": { "type": "boolean" }
                },
                "required": ["from", "to", "type"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let args = parse::<LinkArgs>(&call)?;
        let doc = self
            .store
            .set_link(KnowledgeLinkRequest {
                from: args.from.clone(),
                to: args.to.clone(),
                link_type: parse_link_type(&args.link_type)?,
                remove: args.remove,
            })
            .await?;
        Ok(result(
            call,
            format!(
                "{} link {} {} {}",
                if args.remove { "removed" } else { "set" },
                args.from,
                args.link_type,
                args.to
            ),
            json!({ "document": doc.summary() }),
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
