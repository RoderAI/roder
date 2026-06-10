use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::files::{parse, result};
use crate::workspace::Workspace;

const DESIGN_DIR_NAME: &str = "design";
const DESIGN_VERSION: &str = "0.1";

pub(crate) fn register(registry: &mut ToolRegistry, workspace: Workspace) -> anyhow::Result<()> {
    registry.register(Arc::new(DesignReadTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignBatchGetTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignPatchTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignVariablesTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignSetVariablesTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignSnapshotLayoutTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignSpawnAgentsTool {
        workspace: workspace.clone(),
    }))?;
    registry.register(Arc::new(DesignGuidelinesTool))?;
    registry.register(Arc::new(DesignExportNodesTool { workspace }))
}

#[derive(Debug)]
struct DesignReadTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignBatchGetTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignPatchTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignVariablesTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignSetVariablesTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignSnapshotLayoutTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignSpawnAgentsTool {
    workspace: Workspace,
}

#[derive(Debug)]
struct DesignGuidelinesTool;

#[derive(Debug)]
struct DesignExportNodesTool {
    workspace: Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesignDocument {
    version: String,
    document_id: String,
    title: String,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    nodes: BTreeMap<String, Value>,
    #[serde(default)]
    root_ids: Vec<String>,
    #[serde(default)]
    variables: BTreeMap<String, Value>,
    #[serde(default)]
    assets: BTreeMap<String, Value>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchGetArgs {
    #[serde(default)]
    node_ids: Vec<String>,
    #[serde(default)]
    patterns: Vec<SearchPattern>,
    parent_id: Option<String>,
    read_depth: Option<u32>,
    search_depth: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchPattern {
    name: Option<String>,
    #[serde(rename = "type")]
    node_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetVariablesArgs {
    variables: BTreeMap<String, Value>,
    #[serde(default)]
    replace: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchArgs {
    operations: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportNodesArgs {
    node_ids: Vec<String>,
    output_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnAgentsArgs {
    scope_node_ids: Vec<String>,
    prompt: Option<String>,
    allow_patch: Option<bool>,
    allow_export: Option<bool>,
    require_review: Option<bool>,
}

#[async_trait::async_trait]
impl ToolExecutor for DesignReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_read".to_string(),
            description: "Read or create the project-specific ~/.roder/design/<project>.roderdesign document for the AI-controlled Design canvas.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, document) = load_or_create(&workspace)?;
        Ok(result(
            call,
            format!(
                "Read {} with {} nodes.",
                workspace.display(&path),
                document.nodes.len()
            ),
            json!({ "path": workspace.display(&path), "document": document, "nodeAliases": design_node_aliases(&document) }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignBatchGetTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_batch_get".to_string(),
            description: "Read/search .roderdesign nodes by ids or patterns. Combine reads and keep depths small for large documents.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_ids": { "type": "array", "items": { "type": "string" } },
                    "patterns": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "type": { "type": "string" }
                            },
                            "additionalProperties": false
                        }
                    },
                    "parent_id": { "type": "string" },
                    "read_depth": { "type": "integer", "minimum": 0, "maximum": 5 },
                    "search_depth": { "type": "integer", "minimum": 0, "maximum": 12 }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<BatchGetArgs>(&call)?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, document) = load_or_create(&workspace)?;
        let args = resolve_batch_get_aliases(&document, args);
        let nodes = batch_get(&document, &args);
        Ok(result(
            call,
            format!(
                "Read {} design nodes from {}.",
                nodes.len(),
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "nodes": nodes, "nodeAliases": design_node_aliases(&document) }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignVariablesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_get_variables".to_string(),
            description: "Read variables/tokens from the project-specific ~/.roder/design/<project>.roderdesign document."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, document) = load_or_create(&workspace)?;
        Ok(result(
            call,
            format!(
                "Read {} design variables from {}.",
                document.variables.len(),
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "variables": document.variables }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignSetVariablesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_set_variables".to_string(),
            description: "Merge or replace variables/tokens in the project-specific ~/.roder/design/<project>.roderdesign document. Prefer this over generic design_patch for token-only updates.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "variables": {
                        "type": "object",
                        "description": "Variables/tokens to merge into the design document."
                    },
                    "replace": {
                        "type": "boolean",
                        "description": "When true, clear existing variables before writing the provided variables."
                    }
                },
                "required": ["variables"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<SetVariablesArgs>(&call)?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, mut document) = load_or_create(&workspace)?;
        let count = args.variables.len();
        set_design_variables(&mut document, args.variables, args.replace);
        document.updated_at = now_iso();
        save(&path, &document)?;
        Ok(result(
            call,
            format!(
                "Wrote {count} design variable(s) to {}.",
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "document": document, "applied": 1 }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignSnapshotLayoutTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_snapshot_layout".to_string(),
            description:
                "Read design node layout rectangles and basic layout problems from the project-specific .roderdesign document."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, document) = load_or_create(&workspace)?;
        let nodes = snapshot_layout(&document);
        Ok(result(
            call,
            format!(
                "Read {} layout node(s) from {}.",
                nodes.len(),
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "nodes": nodes }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignSpawnAgentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_spawn_agents".to_string(),
            description: "Plan scoped Roder design subagents for container nodes. Returns validated scope metadata and permission guidance; use the plan before dispatching agent work.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "scope_node_ids": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string" }
                    },
                    "prompt": { "type": "string" },
                    "allow_patch": { "type": "boolean" },
                    "allow_export": { "type": "boolean" },
                    "require_review": { "type": "boolean" }
                },
                "required": ["scope_node_ids"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<SpawnAgentsArgs>(&call)?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, document) = load_or_create(&workspace)?;
        let planned = design_spawn_agent_plan(&document, &args)?;
        let allow_patch = args.allow_patch.unwrap_or(false);
        let allow_export = args.allow_export.unwrap_or(true);
        let require_review = args.require_review.unwrap_or(true);
        let instructions =
            design_spawn_agent_instructions(&args, allow_patch, allow_export, require_review);
        Ok(result(
            call,
            format!(
                "Planned {} scoped design agent(s) for {}.",
                planned.len(),
                workspace.display(&path)
            ),
            json!({
                "path": workspace.display(&path),
                "planned": planned,
                "allowPatch": allow_patch,
                "allowExport": allow_export,
                "requireReview": require_review,
                "instructions": instructions
            }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignGuidelinesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_get_guidelines".to_string(),
            description: "Read Roder Design Canvas guidelines for agent-driven design work."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        _ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let guidelines = design_guidelines();
        Ok(result(
            call,
            "Read Design Canvas guidelines.".to_string(),
            guidelines,
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignExportNodesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_export_nodes".to_string(),
            description: "Export .roderdesign nodes to SVG files. This is the current export format for Design Canvas visuals.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_ids": { "type": "array", "items": { "type": "string" } },
                    "output_dir": { "type": "string", "description": "Optional output directory. Defaults to .roder/design-exports under the workspace." }
                },
                "required": ["node_ids"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<ExportNodesArgs>(&call)?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (_path, document) = load_or_create(&workspace)?;
        let export_dir = args
            .output_dir
            .as_deref()
            .map(|dir| workspace.resolve_for_write(dir))
            .transpose()?
            .unwrap_or_else(|| {
                workspace
                    .resolve_for_write(".roder/design-exports")
                    .expect("default export path resolves")
            });
        std::fs::create_dir_all(&export_dir)?;
        let mut exported = Vec::new();
        for node_id in args.node_ids {
            let node_id = resolve_node_alias(&document, &node_id).unwrap_or(node_id);
            let node = document
                .nodes
                .get(&node_id)
                .with_context(|| format!("unknown nodeId: {node_id}"))?;
            let svg = node_to_svg(&document, node);
            let name = node.get("name").and_then(Value::as_str).unwrap_or(&node_id);
            let output_path =
                export_dir.join(format!("{}-{node_id}.svg", sanitize_file_name(name)));
            std::fs::write(&output_path, svg)?;
            exported.push(json!({ "nodeId": node_id, "path": workspace.display(&output_path) }));
        }
        Ok(result(
            call,
            format!("Exported {} design node(s).", exported.len()),
            json!({ "exported": exported }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_patch".to_string(),
            description: "Apply typed patch operations to the project-specific ~/.roder/design/<project>.roderdesign document. Supports insert_node, update_node, delete_node, and set_variables.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "minItems": 1,
                        "items": design_patch_operation_schema()
                    }
                },
                "required": ["operations"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        ctx.require_workspace()?;
        let args = parse::<PatchArgs>(&call)?;
        let workspace =
            Workspace::local_from_context_or_fallback(&ctx, &self.workspace, "design tools")?;
        let (path, mut document) = load_or_create(&workspace)?;
        let applied = args.operations.len();
        for operation in args.operations {
            let operation = resolve_operation_aliases(&document, operation);
            apply_operation(&mut document, operation)?;
        }
        document.updated_at = now_iso();
        save(&path, &document)?;
        Ok(result(
            call,
            format!(
                "Applied {applied} design operation(s) to {}.",
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "document": document, "applied": applied }),
            false,
        ))
    }
}

fn design_document_path(workspace: &Workspace) -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().context("resolve home directory for design document")?;
    Ok(home
        .join(".roder")
        .join(DESIGN_DIR_NAME)
        .join(design_document_file_name(workspace)))
}

fn design_patch_operation_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "op": { "const": "insert_node" },
                    "parentId": { "type": ["string", "null"], "description": "Optional parent id or simple alias such as n1." },
                    "index": { "type": "integer", "minimum": 0 },
                    "node": { "type": "object", "description": "Complete design node with id, type, name, geometry, and optional childIds." }
                },
                "required": ["op", "node"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "update_node" },
                    "nodeId": { "type": "string", "description": "Canonical node id or simple alias such as n1." },
                    "patch": { "type": "object", "description": "Partial node fields to merge into the target node." }
                },
                "required": ["op", "nodeId", "patch"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "delete_node" },
                    "nodeId": { "type": "string", "description": "Canonical node id or simple alias such as n1." },
                    "recursive": { "type": "boolean", "default": false }
                },
                "required": ["op", "nodeId"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "reorder_node" },
                    "nodeId": { "type": "string", "description": "Canonical node id or simple alias such as n1." },
                    "index": { "type": "integer", "minimum": 0 }
                },
                "required": ["op", "nodeId", "index"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "set_variables" },
                    "variables": { "type": "object" },
                    "replace": { "type": "boolean", "default": false }
                },
                "required": ["op", "variables"],
                "additionalProperties": false
            }
        ]
    })
}

fn design_document_file_name(workspace: &Workspace) -> String {
    let root = workspace.root();
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let slug = slugify_project_name(name);
    let stable = stable_id("project", &root.display().to_string());
    format!("{slug}-{stable}.roderdesign")
}

fn slugify_project_name(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug.chars().take(48).collect()
    }
}

fn load_or_create(workspace: &Workspace) -> anyhow::Result<(PathBuf, DesignDocument)> {
    let path = design_document_path(workspace)?;
    if path.exists() {
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let document =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        return Ok((path, document));
    }
    let document = new_document(workspace);
    save(&path, &document)?;
    Ok((path, document))
}

fn new_document(workspace: &Workspace) -> DesignDocument {
    let now = now_iso();
    let frame_id = "frame-root".to_string();
    let mut nodes = BTreeMap::new();
    nodes.insert(
        frame_id.clone(),
        json!({
            "id": frame_id,
            "type": "frame",
            "name": "Frame",
            "parentId": null,
            "childIds": [],
            "x": 0,
            "y": 0,
            "width": 800,
            "height": 600,
            "visible": true,
            "fill": { "kind": "color", "value": "#ffffff" }
        }),
    );
    DesignDocument {
        version: DESIGN_VERSION.to_string(),
        document_id: stable_id("design", &workspace.root().display().to_string()),
        title: workspace
            .root()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Workspace")
            .to_string(),
        created_at: now.clone(),
        updated_at: now,
        nodes,
        root_ids: vec!["frame-root".to_string()],
        variables: BTreeMap::new(),
        assets: BTreeMap::new(),
        metadata: BTreeMap::from([(
            "workspaceRoot".to_string(),
            Value::String(workspace.root().display().to_string()),
        )]),
    }
}

fn save(path: &PathBuf, document: &DesignDocument) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(document)?;
    let tmp = path.with_extension(format!("roderdesign.tmp.{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn set_design_variables(
    document: &mut DesignDocument,
    variables: BTreeMap<String, Value>,
    replace: bool,
) {
    if replace {
        document.variables.clear();
    }
    document.variables.extend(variables);
}

fn batch_get(document: &DesignDocument, args: &BatchGetArgs) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    if args.node_ids.is_empty() && args.patterns.is_empty() {
        for id in &document.root_ids {
            push_node(
                document,
                id,
                args.read_depth.unwrap_or(1),
                &mut seen,
                &mut result,
            );
        }
        return result;
    }
    for id in &args.node_ids {
        push_node(
            document,
            id,
            args.read_depth.unwrap_or(1),
            &mut seen,
            &mut result,
        );
    }
    if !args.patterns.is_empty() {
        let roots = args
            .parent_id
            .as_ref()
            .map(|id| vec![id.clone()])
            .unwrap_or_else(|| document.root_ids.clone());
        let mut queue = VecDeque::new();
        for id in roots {
            queue.push_back((id, 0_u32));
        }
        let max_depth = args.search_depth.unwrap_or(u32::MAX);
        while let Some((id, depth)) = queue.pop_front() {
            if depth > max_depth {
                continue;
            }
            let Some(node) = document.nodes.get(&id) else {
                continue;
            };
            if args
                .patterns
                .iter()
                .any(|pattern| pattern_matches(node, pattern))
            {
                push_node(
                    document,
                    &id,
                    args.read_depth.unwrap_or(1),
                    &mut seen,
                    &mut result,
                );
            }
            if let Some(children) = node.get("childIds").and_then(Value::as_array) {
                for child_id in children.iter().filter_map(Value::as_str) {
                    queue.push_back((child_id.to_string(), depth.saturating_add(1)));
                }
            }
        }
    }
    result
}

fn design_node_aliases(document: &DesignDocument) -> Vec<Value> {
    document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, (node_id, node))| {
            json!({
                "alias": format!("n{}", index + 1),
                "nodeId": node_id,
                "name": node.get("name").and_then(Value::as_str).unwrap_or_default(),
                "type": node.get("type").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect()
}

fn resolve_node_alias(document: &DesignDocument, id_or_alias: &str) -> Option<String> {
    if document.nodes.contains_key(id_or_alias) {
        return Some(id_or_alias.to_string());
    }
    let alias = id_or_alias.strip_prefix('n')?;
    let index = alias.parse::<usize>().ok()?.checked_sub(1)?;
    document.nodes.keys().nth(index).cloned()
}

fn resolve_batch_get_aliases(document: &DesignDocument, mut args: BatchGetArgs) -> BatchGetArgs {
    args.node_ids = args
        .node_ids
        .into_iter()
        .map(|id| resolve_node_alias(document, &id).unwrap_or(id))
        .collect();
    args.parent_id = args
        .parent_id
        .map(|id| resolve_node_alias(document, &id).unwrap_or(id));
    args
}

fn resolve_operation_aliases(document: &DesignDocument, mut operation: Value) -> Value {
    if let Some(object) = operation.as_object_mut() {
        for key in ["nodeId", "parentId"] {
            if let Some(value) = object.get(key).and_then(Value::as_str).map(str::to_string) {
                if let Some(resolved) = resolve_node_alias(document, &value) {
                    object.insert(key.to_string(), Value::String(resolved));
                }
            }
        }
    }
    operation
}

fn push_node(
    document: &DesignDocument,
    id: &str,
    depth: u32,
    seen: &mut HashSet<String>,
    result: &mut Vec<Value>,
) {
    if !seen.insert(id.to_string()) {
        return;
    }
    let Some(node) = document.nodes.get(id) else {
        return;
    };
    result.push(node.clone());
    if depth == 0 {
        return;
    }
    if let Some(children) = node.get("childIds").and_then(Value::as_array) {
        for child_id in children.iter().filter_map(Value::as_str) {
            push_node(document, child_id, depth - 1, seen, result);
        }
    }
}

fn pattern_matches(node: &Value, pattern: &SearchPattern) -> bool {
    if let Some(expected_type) = &pattern.node_type {
        if node.get("type").and_then(Value::as_str) != Some(expected_type.as_str()) {
            return false;
        }
    }
    if let Some(name) = &pattern.name {
        let actual = node.get("name").and_then(Value::as_str).unwrap_or_default();
        if !actual.to_lowercase().contains(&name.to_lowercase()) {
            return false;
        }
    }
    true
}

fn node_to_svg(document: &DesignDocument, node: &Value) -> String {
    let x = node.get("x").and_then(Value::as_f64).unwrap_or_default();
    let y = node.get("y").and_then(Value::as_f64).unwrap_or_default();
    let width = node
        .get("width")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let height = node
        .get("height")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let mut body = String::new();
    render_node_svg(document, node, -x, -y, &mut body);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

fn render_node_svg(
    document: &DesignDocument,
    node: &Value,
    offset_x: f64,
    offset_y: f64,
    out: &mut String,
) {
    let x = node.get("x").and_then(Value::as_f64).unwrap_or_default() + offset_x;
    let y = node.get("y").and_then(Value::as_f64).unwrap_or_default() + offset_y;
    let width = node
        .get("width")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let height = node
        .get("height")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("frame");
    let opacity = opacity_attr(node.get("opacity").and_then(Value::as_f64));
    let transform = transform_attr(
        node.get("rotation").and_then(Value::as_f64),
        x + width / 2.0,
        y + height / 2.0,
    );
    let corner_radius = corner_radius(node.get("cornerRadius"));
    let fill = paint_color(node.get("fill")).unwrap_or_else(|| {
        if kind == "text" {
            "#18181b".to_string()
        } else {
            "#ffffff".to_string()
        }
    });
    let stroke = paint_color(node.get("stroke")).unwrap_or_else(|| "#d4d4d8".to_string());
    if kind == "component" || kind == "instance" {
        let source_component_id = node
            .get("sourceComponentId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let overrides = node
            .get("overrides")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            r#"<!-- roder-design-node type="{}" id="{}" component-id="{}" source-component-id="{}" overrides="{}" -->"#,
            escape_xml(kind),
            escape_xml(node.get("id").and_then(Value::as_str).unwrap_or("")),
            escape_xml(
                node.get("componentId")
                    .and_then(Value::as_str)
                    .or_else(|| node.get("id").and_then(Value::as_str))
                    .unwrap_or("")
            ),
            escape_xml(source_component_id),
            escape_xml(&overrides)
        ));
    }
    match kind {
        "ellipse" => out.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{}" stroke="{}"{}{} />"#,
            x + width / 2.0,
            y + height / 2.0,
            width / 2.0,
            height / 2.0,
            escape_xml(&fill),
            escape_xml(&stroke),
            opacity,
            transform
        )),
        "line" => out.push_str(&format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"{}{} />"#,
            x,
            y,
            x + width,
            y + height,
            escape_xml(&stroke),
            stroke_width(node.get("stroke")),
            opacity,
            transform
        )),
        "path" => {
            let view_box = node
                .get("viewBox")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("0 0 {} {}", width, height));
            let path_data = node
                .get("pathData")
                .or_else(|| node.get("d"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let path_fill = if fill == "transparent" { "none" } else { fill.as_str() };
            out.push_str(&format!(
                r#"<svg x="{}" y="{}" width="{}" height="{}" viewBox="{}"{}{}><path d="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round" /></svg>"#,
                x,
                y,
                width,
                height,
                escape_xml(&view_box),
                opacity,
                transform,
                escape_xml(path_data),
                escape_xml(path_fill),
                escape_xml(&stroke),
                stroke_width(node.get("stroke"))
            ));
        }
        "icon" => {
            let view_box = node.get("viewBox").and_then(Value::as_str).unwrap_or("0 0 24 24");
            let path_data = node
                .get("svg")
                .or_else(|| node.get("pathData"))
                .or_else(|| node.get("d"))
                .and_then(Value::as_str)
                .unwrap_or("");
            out.push_str(&format!(
                r#"<svg x="{}" y="{}" width="{}" height="{}" viewBox="{}"{}{}><path d="{}" fill="{}" /></svg>"#,
                x,
                y,
                width,
                height,
                escape_xml(view_box),
                opacity,
                transform,
                escape_xml(path_data),
                escape_xml(&fill)
            ));
        }
        "image" => {
            if let Some(src) = node.get("src").and_then(Value::as_str) {
                if !src.is_empty() {
                    out.push_str(&format!(
                        r#"<image x="{}" y="{}" width="{}" height="{}" href="{}" preserveAspectRatio="xMidYMid slice"{}{} />"#,
                        x,
                        y,
                        width,
                        height,
                        escape_xml(src),
                        opacity,
                        transform
                    ));
                } else {
                    render_placeholder_rect(x, y, width, height, &fill, &stroke, corner_radius, &opacity, &transform, out);
                }
            } else {
                render_placeholder_rect(x, y, width, height, &fill, &stroke, corner_radius, &opacity, &transform, out);
            }
        }
        "prompt" => {
            render_placeholder_rect(x, y, width, height, &fill, &stroke, corner_radius, &opacity, &transform, out);
            out.push_str(&format!(
                r##"<text x="{}" y="{}" fill="#92400e" font-family="system-ui, sans-serif" font-size="13" font-weight="600"{}{}>Prompt</text><text x="{}" y="{}" fill="#92400e" font-family="system-ui, sans-serif" font-size="13"{}{}>{}</text>"##,
                x + 12.0,
                y + 24.0,
                opacity,
                transform,
                x + 12.0,
                y + 48.0,
                opacity,
                transform,
                escape_xml(node.get("prompt").or_else(|| node.get("content")).and_then(Value::as_str).unwrap_or("Describe the design change for Roder..."))
            ));
        }
        "text" => {
            let font_size = font_size(node.get("fontSize"));
            let font_weight = font_weight(node.get("fontWeight"));
            let anchor = text_anchor(node.get("textAlign"));
            let text_x = match anchor {
                "middle" => x + width / 2.0,
                "end" => x + width,
                _ => x,
            };
            out.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{}" font-family="system-ui, sans-serif" font-size="{}" font-weight="{}" text-anchor="{}"{}{}>{}</text>"#,
            text_x,
            y + font_size,
            escape_xml(&fill),
            font_size,
            font_weight,
            anchor,
            opacity,
            transform,
            escape_xml(node.get("content").and_then(Value::as_str).or_else(|| node.get("name").and_then(Value::as_str)).unwrap_or("Text"))
        ));
        }
        _ => render_placeholder_rect(x, y, width, height, &fill, &stroke, corner_radius, &opacity, &transform, out),
    }
    if let Some(children) = node.get("childIds").and_then(Value::as_array) {
        for child_id in children.iter().filter_map(Value::as_str) {
            if let Some(child) = document.nodes.get(child_id) {
                render_node_svg(document, child, x, y, out);
            }
        }
    }
}

fn render_placeholder_rect(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    fill: &str,
    stroke: &str,
    corner_radius: f64,
    opacity: &str,
    transform: &str,
    out: &mut String,
) {
    out.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" ry="{}" fill="{}" stroke="{}"{}{} />"#,
        x,
        y,
        width,
        height,
        corner_radius,
        corner_radius,
        escape_xml(fill),
        escape_xml(stroke),
        opacity,
        transform
    ));
}

fn corner_radius(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(8.0)
        .max(0.0)
}

fn transform_attr(rotation: Option<f64>, cx: f64, cy: f64) -> String {
    let Some(rotation) = rotation else {
        return String::new();
    };
    if !rotation.is_finite() || rotation.rem_euclid(360.0) == 0.0 {
        String::new()
    } else {
        format!(
            r#" transform="rotate({} {} {})""#,
            rotation.rem_euclid(360.0),
            cx,
            cy
        )
    }
}

fn opacity_attr(opacity: Option<f64>) -> String {
    let Some(opacity) = opacity else {
        return String::new();
    };
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity >= 1.0 {
        String::new()
    } else {
        format!(r#" opacity="{opacity}""#)
    }
}

fn paint_color(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Object(map) => map.get("value").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn sanitize_file_name(input: &str) -> String {
    let sanitized = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = sanitized
        .trim_matches('-')
        .chars()
        .take(48)
        .collect::<String>();
    if trimmed.is_empty() {
        "node".to_string()
    } else {
        trimmed
    }
}

fn stroke_width(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get("width"))
        .and_then(Value::as_f64)
        .unwrap_or(2.0)
}

fn font_size(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .unwrap_or(16.0)
        .round()
        .clamp(8.0, 144.0)
}

fn font_weight(value: Option<&Value>) -> f64 {
    let weight = value.and_then(Value::as_f64).unwrap_or(500.0);
    ((weight / 100.0).round() * 100.0).clamp(100.0, 900.0)
}

fn text_anchor(value: Option<&Value>) -> &'static str {
    match value.and_then(Value::as_str) {
        Some("center") => "middle",
        Some("right") => "end",
        _ => "start",
    }
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn snapshot_layout(document: &DesignDocument) -> Vec<Value> {
    let mut nodes = document
        .nodes
        .values()
        .map(|node| {
            let width = node.get("width").and_then(Value::as_f64).unwrap_or_default();
            let height = node.get("height").and_then(Value::as_f64).unwrap_or_default();
            let mut problems = Vec::new();
            if width <= 0.0 || height <= 0.0 {
                problems.push("non-positive size");
            }
            json!({
                "id": node.get("id").and_then(Value::as_str).unwrap_or_default(),
                "type": node.get("type").and_then(Value::as_str).unwrap_or_default(),
                "name": node.get("name").and_then(Value::as_str).unwrap_or_default(),
                "parentId": node.get("parentId").cloned().unwrap_or(Value::Null),
                "childIds": node.get("childIds").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
                "x": node.get("x").and_then(Value::as_f64).unwrap_or_default(),
                "y": node.get("y").and_then(Value::as_f64).unwrap_or_default(),
                "width": width,
                "height": height,
                "problems": problems,
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .cmp(&b.get("id").and_then(Value::as_str))
    });
    nodes
}

fn design_spawn_agent_plan(
    document: &DesignDocument,
    args: &SpawnAgentsArgs,
) -> anyhow::Result<Vec<Value>> {
    if args.scope_node_ids.is_empty() {
        bail!("scope_node_ids must include at least one container node id");
    }
    let mut seen = HashSet::new();
    let mut planned = Vec::new();
    for scope_id in &args.scope_node_ids {
        let scope_id = resolve_node_alias(document, scope_id).unwrap_or_else(|| scope_id.clone());
        if !seen.insert(scope_id.clone()) {
            continue;
        }
        let node = document
            .nodes
            .get(&scope_id)
            .with_context(|| format!("unknown scope node id: {scope_id}"))?;
        let node_type = node.get("type").and_then(Value::as_str).unwrap_or("node");
        if !can_scope_design_agent(node_type) {
            bail!(
                "scope node {scope_id} is type {node_type}; expected frame, group, component, or instance"
            );
        }
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&scope_id)
            .to_string();
        let child_count = node
            .get("childIds")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let parent_id = node
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let alias = design_node_aliases(document)
            .into_iter()
            .find(|alias| alias.get("nodeId").and_then(Value::as_str) == Some(scope_id.as_str()))
            .and_then(|alias| {
                alias
                    .get("alias")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| scope_id.clone());
        planned.push(json!({
            "alias": alias,
            "scopeNodeId": scope_id,
            "scopeName": name,
            "scopeType": node_type,
            "parentId": parent_id,
            "childCount": child_count
        }));
    }
    Ok(planned)
}

fn can_scope_design_agent(node_type: &str) -> bool {
    matches!(node_type, "frame" | "group" | "component" | "instance")
}

fn design_spawn_agent_instructions(
    args: &SpawnAgentsArgs,
    allow_patch: bool,
    allow_export: bool,
    require_review: bool,
) -> String {
    let prompt = args
        .prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or("Improve the scoped design container while preserving neighboring frames.");
    format!(
        "{prompt}\nPermissions: allow_patch={allow_patch}, allow_export={allow_export}, require_review={require_review}. Read editor state first, batch-get scoped children before edits, and keep changes inside each scope node."
    )
}

fn design_guidelines() -> Value {
    json!({
        "categories": [
            {
                "name": "workflow",
                "description": "How Roder agents should work with project-specific .roderdesign documents.",
                "guidelines": [
                    "Call design_read before editing.",
                    "Use design_batch_get to combine node reads and searches.",
                    "Apply edits with typed design_patch operations.",
                    "Run design_snapshot_layout after structural edits."
                ]
            },
            {
                "name": "layout",
                "description": "Default product layout guidance for early Design Canvas documents.",
                "guidelines": [
                    "Use frames as artboards or major containers.",
                    "Name frames and important nodes with user-recognizable labels.",
                    "Keep generated designs simple enough to map to real application components."
                ]
            }
        ]
    })
}

fn apply_operation(document: &mut DesignDocument, operation: Value) -> anyhow::Result<()> {
    let op = operation
        .get("op")
        .and_then(Value::as_str)
        .context("operation.op is required")?;
    match op {
        "insert_node" => {
            let mut node = operation.get("node").cloned().context("node is required")?;
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .context("node.id is required")?
                .to_string();
            if document.nodes.contains_key(&id) {
                bail!("node id already exists: {id}");
            }
            let parent_id = operation
                .get("parentId")
                .or_else(|| operation.get("parent_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Value::Object(obj) = &mut node {
                obj.insert(
                    "parentId".to_string(),
                    parent_id.clone().map(Value::String).unwrap_or(Value::Null),
                );
            }
            if let Some(parent_id) = parent_id {
                let parent = document
                    .nodes
                    .get_mut(&parent_id)
                    .with_context(|| format!("unknown parentId: {parent_id}"))?;
                let children = parent
                    .as_object_mut()
                    .context("parent node must be an object")?
                    .entry("childIds")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .context("parent childIds must be an array")?;
                let index = operation
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(children.len())
                    .min(children.len());
                children.insert(index, Value::String(id.clone()));
            } else {
                let index = operation
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(document.root_ids.len())
                    .min(document.root_ids.len());
                document.root_ids.insert(index, id.clone());
            }
            document.nodes.insert(id, node);
        }
        "update_node" => {
            let id = operation
                .get("nodeId")
                .or_else(|| operation.get("node_id"))
                .and_then(Value::as_str)
                .context("nodeId is required")?;
            let patch = operation
                .get("patch")
                .cloned()
                .context("patch is required")?;
            let node = document
                .nodes
                .get_mut(id)
                .with_context(|| format!("unknown nodeId: {id}"))?;
            merge_json(node, patch);
        }
        "delete_node" => {
            let id = operation
                .get("nodeId")
                .or_else(|| operation.get("node_id"))
                .and_then(Value::as_str)
                .context("nodeId is required")?;
            let recursive = operation
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            delete_node(document, id, recursive)?;
        }
        "reorder_node" => {
            let id = operation
                .get("nodeId")
                .or_else(|| operation.get("node_id"))
                .and_then(Value::as_str)
                .context("nodeId is required")?;
            let index = operation
                .get("index")
                .and_then(Value::as_u64)
                .context("index is required")? as usize;
            reorder_node(document, id, index)?;
        }
        "set_variables" => {
            let variables = operation
                .get("variables")
                .and_then(Value::as_object)
                .context("variables object is required")?;
            if operation
                .get("replace")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                document.variables.clear();
            }
            for (key, value) in variables {
                document.variables.insert(key.clone(), value.clone());
            }
        }
        other => bail!("unsupported design operation: {other}"),
    }
    Ok(())
}

fn delete_node(document: &mut DesignDocument, id: &str, recursive: bool) -> anyhow::Result<()> {
    let node = document
        .nodes
        .get(id)
        .cloned()
        .with_context(|| format!("unknown nodeId: {id}"))?;
    let child_ids = node
        .get("childIds")
        .and_then(Value::as_array)
        .map(|children| {
            children
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !recursive && !child_ids.is_empty() {
        bail!("node has children; pass recursive=true");
    }
    for child_id in child_ids {
        delete_node(document, &child_id, true)?;
    }
    if let Some(parent_id) = node.get("parentId").and_then(Value::as_str) {
        if let Some(parent) = document.nodes.get_mut(parent_id) {
            if let Some(children) = parent.get_mut("childIds").and_then(Value::as_array_mut) {
                children.retain(|child| child.as_str() != Some(id));
            }
        }
    } else {
        document.root_ids.retain(|root_id| root_id != id);
    }
    document.nodes.remove(id);
    Ok(())
}

fn reorder_node(document: &mut DesignDocument, id: &str, index: usize) -> anyhow::Result<()> {
    let parent_id = document
        .nodes
        .get(id)
        .with_context(|| format!("unknown nodeId: {id}"))?
        .get("parentId")
        .or_else(|| {
            document
                .nodes
                .get(id)
                .and_then(|node| node.get("parent_id"))
        })
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if let Some(parent_id) = parent_id {
        let parent = document
            .nodes
            .get_mut(&parent_id)
            .with_context(|| format!("unknown parentId: {parent_id}"))?;
        let child_ids = if parent.get("childIds").is_some() {
            parent.get_mut("childIds")
        } else {
            parent.get_mut("child_ids")
        };
        let siblings = child_ids
            .and_then(Value::as_array_mut)
            .context("parent childIds must be an array")?;
        let current_index = siblings
            .iter()
            .position(|value| value.as_str() == Some(id))
            .with_context(|| format!("node is not listed in its parent order: {id}"))?;
        let value = siblings.remove(current_index);
        let insert_at = index.min(siblings.len());
        siblings.insert(insert_at, value);
    } else {
        let current_index = document
            .root_ids
            .iter()
            .position(|value| value == id)
            .with_context(|| format!("node is not listed in root order: {id}"))?;
        let value = document.root_ids.remove(current_index);
        let insert_at = index.min(document.root_ids.len());
        document.root_ids.insert(insert_at, value);
    }
    Ok(())
}

fn merge_json(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch,
    }
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn stable_id(prefix: &str, value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}_{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_nodes_export_svg_image_when_src_is_set() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "image-1".to_string(),
                json!({
                    "id": "image-1",
                    "type": "image",
                    "name": "Hero",
                    "x": 10,
                    "y": 20,
                    "width": 300,
                    "height": 180,
                    "rotation": 15,
                    "opacity": 0.5,
                    "src": "https://example.com/cat.png"
                }),
            )]),
            root_ids: vec!["image-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("image-1").unwrap());
        assert!(svg.contains("<image "));
        assert!(svg.contains(r#"href="https://example.com/cat.png""#));
        assert!(svg.contains(r#"opacity="0.5""#));
        assert!(svg.contains(r#"transform="rotate(15 "#));
        assert!(svg.contains(r#"preserveAspectRatio="xMidYMid slice""#));
    }

    #[test]
    fn node_aliases_resolve_for_tool_reads_and_patches() {
        let mut document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "frame-root".to_string(),
                json!({ "id": "frame-root", "type": "frame", "name": "Frame", "x": 0, "y": 0, "width": 800, "height": 600 }),
            )]),
            root_ids: vec!["frame-root".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        assert_eq!(
            resolve_node_alias(&document, "n1"),
            Some("frame-root".to_string())
        );
        let operation = resolve_operation_aliases(
            &document,
            json!({ "op": "update_node", "nodeId": "n1", "patch": { "name": "Hero" } }),
        );
        apply_operation(&mut document, operation).unwrap();
        assert_eq!(
            document.nodes["frame-root"]
                .get("name")
                .and_then(Value::as_str),
            Some("Hero")
        );
    }

    #[test]
    fn line_nodes_export_svg_line() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "line-1".to_string(),
                json!({
                    "id": "line-1",
                    "type": "line",
                    "name": "Divider",
                    "x": 10,
                    "y": 20,
                    "width": 100,
                    "height": 50,
                    "rotation": 90,
                    "opacity": 0.25,
                    "stroke": { "kind": "color", "value": "#111827", "width": 3 }
                }),
            )]),
            root_ids: vec!["line-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("line-1").unwrap());
        assert!(svg.contains("<line "));
        assert!(svg.contains(r#"x1="0""#));
        assert!(svg.contains(r#"x2="100""#));
        assert!(svg.contains(r##"stroke="#111827""##));
        assert!(svg.contains(r#"stroke-width="3""#));
        assert!(svg.contains(r#"opacity="0.25""#));
        assert!(svg.contains(r#"transform="rotate(90 "#));
    }

    #[test]
    fn rectangle_nodes_export_corner_radius() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "rect-1".to_string(),
                json!({
                    "id": "rect-1",
                    "type": "rectangle",
                    "name": "Card",
                    "x": 10,
                    "y": 20,
                    "width": 100,
                    "height": 50,
                    "cornerRadius": 18,
                    "fill": { "kind": "color", "value": "#ffffff" },
                    "stroke": { "kind": "color", "value": "#111827" }
                }),
            )]),
            root_ids: vec!["rect-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("rect-1").unwrap());
        assert!(svg.contains("<rect "));
        assert!(svg.contains(r#"rx="18""#));
        assert!(svg.contains(r#"ry="18""#));
    }

    #[test]
    fn path_and_icon_nodes_export_vector_svg() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([
                (
                    "path-1".to_string(),
                    json!({
                        "id": "path-1",
                        "type": "path",
                        "name": "Curve",
                        "x": 10,
                        "y": 20,
                        "width": 160,
                        "height": 80,
                        "pathData": "M0 80 C40 0 120 0 160 80",
                        "viewBox": "0 0 160 80",
                        "fill": { "kind": "color", "value": "transparent" },
                        "stroke": { "kind": "color", "value": "#111827", "width": 3 }
                    }),
                ),
                (
                    "icon-1".to_string(),
                    json!({
                        "id": "icon-1",
                        "type": "icon",
                        "name": "Star",
                        "x": 20,
                        "y": 30,
                        "width": 24,
                        "height": 24,
                        "svg": "M12 2l3 7h7l-5.5 4.5L18 21l-6-4-6 4 1.5-7.5L2 9h7z",
                        "viewBox": "0 0 24 24",
                        "fill": { "kind": "color", "value": "#f59e0b" }
                    }),
                ),
            ]),
            root_ids: vec!["path-1".to_string(), "icon-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let path_svg = node_to_svg(&document, document.nodes.get("path-1").unwrap());
        let icon_svg = node_to_svg(&document, document.nodes.get("icon-1").unwrap());
        assert!(path_svg.contains(r#"viewBox="0 0 160 80""#));
        assert!(path_svg.contains(r#"d="M0 80 C40 0 120 0 160 80""#));
        assert!(path_svg.contains(r#"stroke-width="3""#));
        assert!(icon_svg.contains(r#"viewBox="0 0 24 24""#));
        assert!(icon_svg.contains(r##"fill="#f59e0b""##));
    }

    #[test]
    fn prompt_nodes_export_prompt_card() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "prompt-1".to_string(),
                json!({
                    "id": "prompt-1",
                    "type": "prompt",
                    "name": "Prompt",
                    "x": 10,
                    "y": 20,
                    "width": 220,
                    "height": 120,
                    "fill": { "kind": "color", "value": "#fef3c7" },
                    "stroke": { "kind": "color", "value": "#f59e0b" },
                    "prompt": "Make the hero card warmer & clearer"
                }),
            )]),
            root_ids: vec!["prompt-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("prompt-1").unwrap());
        assert!(svg.contains("<rect "));
        assert!(svg.contains(">Prompt</text>"));
        assert!(svg.contains("Make the hero card warmer &amp; clearer"));
    }

    #[test]
    fn text_nodes_export_typography() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "text-1".to_string(),
                json!({
                    "id": "text-1",
                    "type": "text",
                    "name": "Headline",
                    "x": 10,
                    "y": 20,
                    "width": 200,
                    "height": 80,
                    "content": "Hello",
                    "fontSize": 32,
                    "fontWeight": 700,
                    "textAlign": "center",
                    "fill": { "kind": "color", "value": "#111827" }
                }),
            )]),
            root_ids: vec!["text-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("text-1").unwrap());
        assert!(svg.contains("<text "));
        assert!(svg.contains(r#"font-size="32""#));
        assert!(svg.contains(r#"font-weight="700""#));
        assert!(svg.contains(r#"text-anchor="middle""#));
        assert!(svg.contains(r#"x="100""#));
        assert!(svg.contains("Hello"));
    }

    #[test]
    fn component_nodes_export_identity_metadata() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "instance-1".to_string(),
                json!({
                    "id": "instance-1",
                    "type": "instance",
                    "name": "Card Instance",
                    "x": 10,
                    "y": 20,
                    "width": 200,
                    "height": 120,
                    "componentId": "component:card",
                    "sourceComponentId": "component-1",
                    "overrides": ["fill", "width"],
                    "fill": { "kind": "color", "value": "#ffffff" }
                }),
            )]),
            root_ids: vec!["instance-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let svg = node_to_svg(&document, document.nodes.get("instance-1").unwrap());
        assert!(svg.contains(r#"roder-design-node type="instance" id="instance-1" component-id="component:card" source-component-id="component-1" overrides="fill,width""#));
        assert!(svg.contains("<rect "));
    }

    #[test]
    fn set_design_variables_merges_and_replaces_tokens() {
        let mut document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::new(),
            root_ids: Vec::new(),
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        set_design_variables(
            &mut document,
            BTreeMap::from([(
                "color.primary".to_string(),
                json!({ "kind": "color", "value": "#2563eb" }),
            )]),
            false,
        );
        assert!(document.variables.contains_key("color.primary"));

        set_design_variables(
            &mut document,
            BTreeMap::from([(
                "space.4".to_string(),
                json!({ "kind": "spacing", "value": 16 }),
            )]),
            true,
        );
        assert!(!document.variables.contains_key("color.primary"));
        assert!(document.variables.contains_key("space.4"));
    }

    #[test]
    fn design_patch_schema_requires_operations_not_top_level_patch() {
        let workspace = Workspace::new(std::env::temp_dir()).unwrap();
        let schema = DesignPatchTool { workspace }.spec().parameters;
        assert_eq!(schema["required"], serde_json::json!(["operations"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert!(schema["properties"].get("patch").is_none());
        let op_schema = &schema["properties"]["operations"]["items"]["oneOf"];
        assert!(op_schema.as_array().is_some_and(|items| items.len() >= 5));
        assert!(op_schema.to_string().contains("update_node"));
        assert!(op_schema.to_string().contains("nodeId"));
    }

    #[test]
    fn design_document_path_uses_home_roder_design_project_file() {
        let root = std::env::temp_dir().join(format!(
            "Roder Design Test {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let path = design_document_path(&workspace).unwrap();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap();
        assert!(file_name.starts_with("roder-design-test-"));
        assert!(file_name.contains("-project_"));
        assert!(file_name.ends_with(".roderdesign"));
        assert!(
            path.parent()
                .is_some_and(|parent| parent.ends_with(".roder/design"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reorder_node_updates_root_and_child_order() {
        let mut document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([
                (
                    "frame-a".to_string(),
                    json!({ "id": "frame-a", "type": "frame", "name": "A", "childIds": ["text-a", "text-b"] }),
                ),
                (
                    "frame-b".to_string(),
                    json!({ "id": "frame-b", "type": "frame", "name": "B", "childIds": [] }),
                ),
                (
                    "text-a".to_string(),
                    json!({ "id": "text-a", "type": "text", "name": "A", "parentId": "frame-a", "childIds": [] }),
                ),
                (
                    "text-b".to_string(),
                    json!({ "id": "text-b", "type": "text", "name": "B", "parentId": "frame-a", "childIds": [] }),
                ),
            ]),
            root_ids: vec!["frame-a".to_string(), "frame-b".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        reorder_node(&mut document, "frame-b", 0).unwrap();
        assert_eq!(document.root_ids, vec!["frame-b", "frame-a"]);
        reorder_node(&mut document, "text-a", 1).unwrap();
        assert_eq!(
            document.nodes["frame-a"]
                .get("childIds")
                .and_then(Value::as_array),
            Some(&vec![json!("text-b"), json!("text-a")])
        );
    }

    #[test]
    fn spawn_agents_plans_container_scopes() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([
                (
                    "frame-1".to_string(),
                    json!({
                        "id": "frame-1",
                        "type": "frame",
                        "name": "Hero frame",
                        "parentId": null,
                        "childIds": ["text-1"],
                        "x": 0,
                        "y": 0,
                        "width": 400,
                        "height": 240
                    }),
                ),
                (
                    "text-1".to_string(),
                    json!({
                        "id": "text-1",
                        "type": "text",
                        "name": "Headline",
                        "parentId": "frame-1",
                        "childIds": [],
                        "x": 24,
                        "y": 24,
                        "width": 200,
                        "height": 48
                    }),
                ),
            ]),
            root_ids: vec!["frame-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let plan = design_spawn_agent_plan(
            &document,
            &SpawnAgentsArgs {
                scope_node_ids: vec!["frame-1".to_string()],
                prompt: Some("Polish hero".to_string()),
                allow_patch: Some(true),
                allow_export: Some(true),
                require_review: Some(false),
            },
        )
        .unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].get("scopeNodeId").and_then(Value::as_str),
            Some("frame-1")
        );
        assert_eq!(
            plan[0].get("scopeName").and_then(Value::as_str),
            Some("Hero frame")
        );
        assert_eq!(plan[0].get("childCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn spawn_agents_rejects_non_container_scope() {
        let document = DesignDocument {
            version: "0.1".to_string(),
            document_id: "doc".to_string(),
            title: "Test".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            nodes: BTreeMap::from([(
                "text-1".to_string(),
                json!({
                    "id": "text-1",
                    "type": "text",
                    "name": "Headline",
                    "childIds": []
                }),
            )]),
            root_ids: vec!["text-1".to_string()],
            variables: BTreeMap::new(),
            assets: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };
        let error = design_spawn_agent_plan(
            &document,
            &SpawnAgentsArgs {
                scope_node_ids: vec!["text-1".to_string()],
                prompt: None,
                allow_patch: None,
                allow_export: None,
                require_review: None,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("expected frame, group, component, or instance"));
    }
}
