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

const DESIGN_PATH: &str = ".roderdesign";
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
    registry.register(Arc::new(DesignSnapshotLayoutTool {
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
struct DesignSnapshotLayoutTool {
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
struct PatchArgs {
    operations: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportNodesArgs {
    node_ids: Vec<String>,
    output_dir: Option<String>,
}

#[async_trait::async_trait]
impl ToolExecutor for DesignReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_read".to_string(),
            description: "Read or create the workspace .roderdesign document for the AI-controlled Design canvas.".to_string(),
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
        let (path, document) = load_or_create(&workspace)?;
        Ok(result(
            call,
            format!(
                "Read {} with {} nodes.",
                workspace.display(&path),
                document.nodes.len()
            ),
            json!({ "path": workspace.display(&path), "document": document }),
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
        let (path, document) = load_or_create(&workspace)?;
        let nodes = batch_get(&document, &args);
        Ok(result(
            call,
            format!(
                "Read {} design nodes from {}.",
                nodes.len(),
                workspace.display(&path)
            ),
            json!({ "path": workspace.display(&path), "nodes": nodes }),
            false,
        ))
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DesignVariablesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_get_variables".to_string(),
            description: "Read variables/tokens from the workspace .roderdesign document."
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
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
impl ToolExecutor for DesignSnapshotLayoutTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "design_snapshot_layout".to_string(),
            description:
                "Read design node layout rectangles and basic layout problems from .roderdesign."
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
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
            description: "Apply typed patch operations to the workspace .roderdesign document. Supports insert_node, update_node, delete_node, and set_variables.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "object" }
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
        let workspace = Workspace::from_context_or_fallback(&ctx, &self.workspace)?;
        let (path, mut document) = load_or_create(&workspace)?;
        let applied = args.operations.len();
        for operation in args.operations {
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

fn load_or_create(workspace: &Workspace) -> anyhow::Result<(PathBuf, DesignDocument)> {
    let path = workspace.resolve_for_write(DESIGN_PATH)?;
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
    let text = serde_json::to_string_pretty(document)?;
    let tmp = path.with_extension(format!("roderdesign.tmp.{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)?;
    Ok(())
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
    let fill = paint_color(node.get("fill")).unwrap_or_else(|| {
        if kind == "text" {
            "#18181b".to_string()
        } else {
            "#ffffff".to_string()
        }
    });
    let stroke = paint_color(node.get("stroke")).unwrap_or_else(|| "#d4d4d8".to_string());
    match kind {
        "ellipse" => out.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{}" stroke="{}"/>"#,
            x + width / 2.0,
            y + height / 2.0,
            width / 2.0,
            height / 2.0,
            escape_xml(&fill),
            escape_xml(&stroke)
        )),
        "line" => out.push_str(&format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"/>"#,
            x,
            y,
            x + width,
            y + height,
            escape_xml(&stroke),
            stroke_width(node.get("stroke"))
        )),
        "image" => {
            if let Some(src) = node.get("src").and_then(Value::as_str) {
                if !src.is_empty() {
                    out.push_str(&format!(
                        r#"<image x="{}" y="{}" width="{}" height="{}" href="{}" preserveAspectRatio="xMidYMid slice"/>"#,
                        x,
                        y,
                        width,
                        height,
                        escape_xml(src)
                    ));
                } else {
                    render_placeholder_rect(x, y, width, height, &fill, &stroke, out);
                }
            } else {
                render_placeholder_rect(x, y, width, height, &fill, &stroke, out);
            }
        }
        "text" => out.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{}" font-family="system-ui, sans-serif" font-size="16">{}</text>"#,
            x,
            y + 18.0,
            escape_xml(&fill),
            escape_xml(node.get("content").and_then(Value::as_str).or_else(|| node.get("name").and_then(Value::as_str)).unwrap_or("Text"))
        )),
        _ => render_placeholder_rect(x, y, width, height, &fill, &stroke, out),
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
    out: &mut String,
) {
    out.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="8" fill="{}" stroke="{}"/>"#,
        x,
        y,
        width,
        height,
        escape_xml(fill),
        escape_xml(stroke)
    ));
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

fn design_guidelines() -> Value {
    json!({
        "categories": [
            {
                "name": "workflow",
                "description": "How Roder agents should work with .roderdesign documents.",
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
        assert!(svg.contains(r#"preserveAspectRatio="xMidYMid slice""#));
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
    }
}
