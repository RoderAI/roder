use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    Document, ListOptions, RoadmapRuntime, Task, list_documents, parse_document, validate_document,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoadmapToolActivation {
    Inactive,
    RoadmappingMode,
    ExplicitRequest,
}

impl RoadmapToolActivation {
    fn enabled(self) -> bool {
        matches!(
            self,
            RoadmapToolActivation::RoadmappingMode | RoadmapToolActivation::ExplicitRequest
        )
    }
}

#[derive(Debug, Clone)]
pub struct RoadmapToolContributor {
    workspace: PathBuf,
    data_dir: PathBuf,
    activation: RoadmapToolActivation,
}

impl RoadmapToolContributor {
    pub fn new(
        workspace: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        activation: RoadmapToolActivation,
    ) -> Self {
        Self {
            workspace: workspace.into(),
            data_dir: data_dir.into(),
            activation,
        }
    }
}

impl ToolContributor for RoadmapToolContributor {
    fn id(&self) -> ToolProviderId {
        "roadmap-tools".to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        if !self.activation.enabled() {
            return Ok(());
        }
        let config = RoadmapToolConfig {
            workspace: self.workspace.clone(),
            data_dir: self.data_dir.clone(),
        };
        registry.register(Arc::new(RoadmapListTool(config.clone())))?;
        registry.register(Arc::new(RoadmapReadTool(config.clone())))?;
        registry.register(Arc::new(RoadmapCreateTool(config.clone())))?;
        registry.register(Arc::new(RoadmapPatchTool(config.clone())))?;
        registry.register(Arc::new(RoadmapSetTaskStateTool(config.clone())))?;
        registry.register(Arc::new(RoadmapValidateTool(config.clone())))?;
        registry.register(Arc::new(RoadmapThreadListTool(config.clone())))?;
        registry.register(Arc::new(RoadmapThreadSpawnTool(config.clone())))?;
        registry.register(Arc::new(RoadmapThreadAttachTool(config)))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct RoadmapToolConfig {
    workspace: PathBuf,
    data_dir: PathBuf,
}

struct RoadmapListTool(RoadmapToolConfig);
struct RoadmapReadTool(RoadmapToolConfig);
struct RoadmapCreateTool(RoadmapToolConfig);
struct RoadmapPatchTool(RoadmapToolConfig);
struct RoadmapSetTaskStateTool(RoadmapToolConfig);
struct RoadmapValidateTool(RoadmapToolConfig);
struct RoadmapThreadListTool(RoadmapToolConfig);
struct RoadmapThreadSpawnTool(RoadmapToolConfig);
struct RoadmapThreadAttachTool(RoadmapToolConfig);

#[async_trait::async_trait]
impl ToolExecutor for RoadmapListTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_list",
            "List roadmap Markdown documents in the workspace roadmap directory.",
            json!({
                "type": "object",
                "properties": {
                    "include_index": { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapListArgs>(&call)?;
        let documents = list_documents(
            &self.0.workspace,
            ListOptions {
                include_index: args.include_index.unwrap_or(false),
            },
        )?;
        Ok(result(
            call,
            format!("found {} roadmap documents", documents.len()),
            json!({ "documents": documents }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapReadTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_read",
            "Read and parse one roadmap Markdown document.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapPathArgs>(&call)?;
        let path = resolve_roadmap_path(&self.0.workspace, &args.path)?;
        let document = read_document(&path)?;
        Ok(document_result(call, "read roadmap", &document, json!({})))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapCreateTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_create",
            "Create a new roadmap Markdown document using the next phase number.",
            json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string" },
                    "title": { "type": "string" },
                    "goal": { "type": "string" }
                },
                "required": ["slug", "title"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapCreateArgs>(&call)?;
        let slug = sanitize_slug(&args.slug)?;
        let roadmap_dir = self.0.workspace.join("roadmap");
        fs::create_dir_all(&roadmap_dir)?;
        let phase = next_phase_number(&roadmap_dir)?;
        let path = roadmap_dir.join(format!("{phase:02}-{slug}.md"));
        if path.exists() {
            anyhow::bail!("roadmap already exists: {}", path.display());
        }
        let title = args.title.trim();
        if title.is_empty() {
            anyhow::bail!("title must not be empty");
        }
        let goal = args
            .goal
            .as_deref()
            .filter(|goal| !goal.trim().is_empty())
            .unwrap_or("Describe the intended outcome.");
        let content = format!(
            "# {title} Implementation Plan\n\n**Goal:** {goal}\n**Architecture:** Document the architecture before implementation.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Create: `roadmap/{phase:02}-{slug}.md`\n\n## Tasks\n\n- [ ] Draft the implementation plan\n\nRun:\n\n```sh\ncargo test -p roder-roadmap\n```\n\nAcceptance:\n- The roadmap is actionable and validated.\n\n## Phase Acceptance\n\n- [ ] Plan is complete.\n"
        );
        fs::write(&path, content)?;
        let document = read_document(&path)?;
        Ok(document_result(
            call,
            "created roadmap",
            &document,
            json!({ "changed_path": document.path }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapPatchTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_patch",
            "Replace exact text in a roadmap document or the repo-local roadmap-planning skill.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapPatchArgs>(&call)?;
        if args.old_string.is_empty() {
            anyhow::bail!("old_string must not be empty");
        }
        let path = resolve_allowed_write_path(&self.0.workspace, &args.path)?;
        let content = fs::read_to_string(&path)?;
        let replacements = content.matches(&args.old_string).count();
        if replacements == 0 {
            return Ok(result(
                call,
                "old_string does not match file".to_string(),
                json!({ "changed_path": path, "replacements": 0 }),
                true,
            ));
        }
        fs::write(&path, content.replace(&args.old_string, &args.new_string))?;
        let document = if is_roadmap_file(&self.0.workspace, &path) {
            Some(read_document(&path)?)
        } else {
            None
        };
        Ok(tool_result_for_optional_document(
            call,
            "patched roadmap text",
            document.as_ref(),
            json!({ "changed_path": path, "replacements": replacements }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapSetTaskStateTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_set_task_state",
            "Mark a roadmap task done or open. Marking done requires non-empty evidence.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "task_id": { "type": "string" },
                    "checked": { "type": "boolean" },
                    "evidence": { "type": "string" }
                },
                "required": ["path", "task_id", "checked"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapSetTaskStateArgs>(&call)?;
        let evidence = args.evidence.unwrap_or_default();
        if args.checked && evidence.trim().is_empty() {
            anyhow::bail!("evidence is required when marking a roadmap task done");
        }
        let mut runtime = runtime(&self.0);
        runtime.set_roadmap_task(&args.path, &args.task_id, args.checked, &evidence)?;
        let path = resolve_roadmap_path(&self.0.workspace, &args.path)?;
        let document = read_document(&path)?;
        Ok(document_result(
            call,
            "updated roadmap task state",
            &document,
            json!({ "changed_path": document.path, "task_id": args.task_id }),
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapValidateTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_validate",
            "Validate one roadmap document and return diagnostics.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapPathArgs>(&call)?;
        let path = resolve_roadmap_path(&self.0.workspace, &args.path)?;
        let document = read_document(&path)?;
        let validation = validate_document(&document);
        Ok(result(
            call,
            format!("validation diagnostics: {}", validation.diagnostics.len()),
            json!({
                "path": document.path,
                "document_id": document.id,
                "diagnostics": validation.diagnostics,
                "next_unchecked_task": next_unchecked_task(&document),
            }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapThreadListTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_thread_list",
            "List thread attachments for a roadmap document.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapPathArgs>(&call)?;
        let runtime = runtime(&self.0);
        let threads = runtime.list_roadmap_threads(&args.path)?;
        Ok(result(
            call,
            format!("found {} roadmap thread attachments", threads.len()),
            json!({ "threads": threads }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapThreadSpawnTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_thread_spawn",
            "Create a new thread attachment for a roadmap task without mutating transcript history.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "task_id": { "type": "string" }
                },
                "required": ["path", "task_id"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapThreadTaskArgs>(&call)?;
        let mut runtime = runtime(&self.0);
        let attachment = runtime.spawn_roadmap_thread(&args.path, &args.task_id)?;
        Ok(result(
            call,
            "spawned roadmap thread attachment".to_string(),
            json!({ "thread": attachment, "task_id": args.task_id }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RoadmapThreadAttachTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "roadmap_thread_attach",
            "Attach an existing thread id to a roadmap task without mutating transcript history.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "task_id": { "type": "string" },
                    "thread_id": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["path", "task_id", "thread_id"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        require_workspace(&ctx)?;
        let args = parse::<RoadmapThreadAttachArgs>(&call)?;
        let mut runtime = runtime(&self.0);
        let attachment = runtime.attach_roadmap_thread(
            &args.path,
            &args.task_id,
            &args.thread_id,
            args.title,
        )?;
        Ok(result(
            call,
            "attached roadmap thread".to_string(),
            json!({ "thread": attachment, "task_id": args.task_id }),
            false,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RoadmapListArgs {
    include_index: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RoadmapPathArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct RoadmapCreateArgs {
    slug: String,
    title: String,
    goal: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoadmapPatchArgs {
    path: String,
    old_string: String,
    new_string: String,
}

#[derive(Debug, Deserialize)]
struct RoadmapSetTaskStateArgs {
    path: String,
    task_id: String,
    checked: bool,
    evidence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoadmapThreadTaskArgs {
    path: String,
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct RoadmapThreadAttachArgs {
    path: String,
    task_id: String,
    thread_id: String,
    title: Option<String>,
}

fn runtime(config: &RoadmapToolConfig) -> RoadmapRuntime {
    RoadmapRuntime::new(&config.workspace, &config.data_dir)
}

fn parse<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> anyhow::Result<T> {
    serde_json::from_value(call.arguments.clone())
        .map_err(|err| anyhow::anyhow!("invalid {} arguments: {err}", call.name))
}

fn spec(name: &str, description: &str, parameters: serde_json::Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

fn result(call: ToolCall, text: String, data: serde_json::Value, is_error: bool) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text,
        data,
        is_error,
    }
}

fn document_result(
    call: ToolCall,
    text: &str,
    document: &Document,
    extra: serde_json::Value,
) -> ToolResult {
    tool_result_for_optional_document(call, text, Some(document), extra, false)
}

fn tool_result_for_optional_document(
    call: ToolCall,
    text: &str,
    document: Option<&Document>,
    extra: serde_json::Value,
    is_error: bool,
) -> ToolResult {
    let mut data = match extra {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let Some(document) = document {
        data.insert("document".to_string(), json!(document));
        data.insert("path".to_string(), json!(document.path));
        data.insert("document_id".to_string(), json!(document.id));
        data.insert(
            "diagnostics".to_string(),
            json!(validate_document(document).diagnostics),
        );
        data.insert(
            "next_unchecked_task".to_string(),
            json!(next_unchecked_task(document)),
        );
    }
    result(
        call,
        text.to_string(),
        serde_json::Value::Object(data),
        is_error,
    )
}

fn read_document(path: &Path) -> anyhow::Result<Document> {
    let content = fs::read_to_string(path)?;
    Ok(parse_document(path, &content))
}

fn resolve_roadmap_path(workspace: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let workspace = workspace.canonicalize()?;
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    let candidate = candidate.canonicalize()?;
    if is_roadmap_file(&workspace, &candidate) {
        Ok(candidate)
    } else {
        anyhow::bail!(
            "roadmap path must be under {}",
            workspace.join("roadmap").display()
        )
    }
}

fn resolve_allowed_write_path(workspace: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let workspace = workspace.canonicalize()?;
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    let candidate = candidate.canonicalize()?;
    if is_roadmap_file(&workspace, &candidate)
        || candidate.starts_with(workspace.join(".agents/skills/roadmap-planning"))
    {
        Ok(candidate)
    } else {
        anyhow::bail!(
            "roadmap write tools are limited to roadmap/*.md and .agents/skills/roadmap-planning"
        )
    }
}

fn is_roadmap_file(workspace: &Path, path: &Path) -> bool {
    path.parent() == Some(&workspace.join("roadmap"))
        && path.extension().and_then(|ext| ext.to_str()) == Some("md")
}

fn require_workspace(ctx: &ToolExecutionContext) -> anyhow::Result<()> {
    ctx.require_workspace().map(|_| ())
}

fn sanitize_slug(slug: &str) -> anyhow::Result<String> {
    let slug = slug.trim();
    if slug.is_empty() {
        anyhow::bail!("slug must not be empty");
    }
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        anyhow::bail!("slug must contain only lowercase letters, digits, and hyphens");
    }
    Ok(slug.to_string())
}

fn next_phase_number(roadmap_dir: &Path) -> anyhow::Result<u32> {
    let mut max_phase = 0;
    for entry in fs::read_dir(roadmap_dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(prefix) = name.split_once('-').map(|(prefix, _)| prefix) else {
            continue;
        };
        if let Ok(phase) = prefix.parse::<u32>() {
            max_phase = max_phase.max(phase);
        }
    }
    Ok(max_phase + 1)
}

fn next_unchecked_task(document: &Document) -> Option<&Task> {
    document.tasks.iter().find(|task| !task.checked)
}
