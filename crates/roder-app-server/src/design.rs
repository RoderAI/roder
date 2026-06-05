use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;

use roder_protocol::{
    DesignBatchGetParams, DesignBatchGetResult, DesignDocumentResult, DesignEditorStateResult,
    DesignExportNodesParams, DesignExportNodesResult, DesignExportedNode,
    DesignGetEditorStateParams, DesignGetGuidelinesParams, DesignGetVariablesParams,
    DesignGuidelineCategory, DesignGuidelinesResult, DesignLayoutNode, DesignNodeSearchPattern,
    DesignPatchOperation, DesignPatchParams, DesignPatchResult, DesignSnapshotLayoutParams,
    DesignSnapshotLayoutResult, DesignVariablesResult, DesignWorkspaceParams, JsonRpcError,
    RoderDesignDocument, RoderDesignMetadata, RoderDesignNode,
};
use time::OffsetDateTime;

use crate::AppServer;

const DESIGN_FILE_NAME: &str = ".roderdesign";
const DESIGN_VERSION: &str = "0.1";

impl AppServer {
    pub(crate) async fn handle_design_get_editor_state(
        &self,
        params: DesignGetEditorStateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        Ok(serde_json::to_value(DesignEditorStateResult {
            path: path.display().to_string(),
            document,
            schema: params.include_schema.then(design_schema),
            rules: params.include_schema.then(|| DESIGN_RULES.to_string()),
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_read(
        &self,
        params: DesignWorkspaceParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        Ok(serde_json::to_value(DesignDocumentResult {
            path: path.display().to_string(),
            document,
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_batch_get(
        &self,
        params: DesignBatchGetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let nodes = batch_get_nodes(&document, &params);
        Ok(serde_json::to_value(DesignBatchGetResult {
            path: path.display().to_string(),
            nodes,
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_get_variables(
        &self,
        params: DesignGetVariablesParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        Ok(serde_json::to_value(DesignVariablesResult {
            path: path.display().to_string(),
            variables: document.variables,
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_snapshot_layout(
        &self,
        params: DesignSnapshotLayoutParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        Ok(serde_json::to_value(DesignSnapshotLayoutResult {
            path: path.display().to_string(),
            nodes: snapshot_layout(&document),
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_get_guidelines(
        &self,
        _params: DesignGetGuidelinesParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(design_guidelines()).unwrap())
    }

    pub(crate) async fn handle_design_export_nodes(
        &self,
        params: DesignExportNodesParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params
            .format
            .as_deref()
            .is_some_and(|format| format != "svg")
        {
            return Err(invalid_params("only svg export is supported in this phase"));
        }
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let export_dir = export_directory(&path, params.output_dir.as_deref()).await?;
        let mut exported = Vec::new();
        for node_id in params.node_ids {
            let node = document
                .nodes
                .get(&node_id)
                .ok_or_else(|| invalid_params(format!("unknown nodeId: {node_id}")))?;
            let svg = node_to_svg(&document, node);
            let file_name = format!("{}-{}.svg", sanitize_file_name(&node.name), node.id);
            let output_path = export_dir.join(file_name);
            tokio::fs::write(&output_path, svg)
                .await
                .map_err(|err| internal_error(format!("write design export: {err}")))?;
            exported.push(DesignExportedNode {
                node_id: node.id.clone(),
                path: output_path.display().to_string(),
            });
        }
        Ok(serde_json::to_value(DesignExportNodesResult { exported }).unwrap())
    }

    pub(crate) async fn handle_design_patch(
        &self,
        params: DesignPatchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, mut document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let applied = params.operations.len();
        for operation in params.operations {
            apply_operation(&mut document, operation)?;
        }
        document.updated_at = now_iso();
        save_design_document(&path, &document).await?;
        let _ = self
            .protocol_notifications
            .send(roder_protocol::JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "design/documentChanged".to_string(),
                params: serde_json::json!({
                    "path": path.display().to_string(),
                    "workspaceId": params.workspace_id,
                    "rootId": params.root_id,
                    "documentId": document.document_id,
                }),
            });
        Ok(serde_json::to_value(DesignPatchResult {
            path: path.display().to_string(),
            document,
            applied,
        })
        .unwrap())
    }

    async fn load_or_create_design_document(
        &self,
        workspace_id: &str,
        root_id: Option<&str>,
    ) -> Result<(PathBuf, RoderDesignDocument), JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        let resolved = self
            .workspaces
            .resolve_root(runtime_workspace, workspace_id, root_id)
            .await?;
        let root_path = PathBuf::from(&resolved.root.path);
        let path = root_path.join(DESIGN_FILE_NAME);
        if path.exists() {
            let data = tokio::fs::read(&path)
                .await
                .map_err(|err| internal_error(format!("read .roderdesign: {err}")))?;
            let document = serde_json::from_slice(&data)
                .map_err(|err| invalid_params(format!("parse .roderdesign: {err}")))?;
            return Ok((path, document));
        }
        let document = new_design_document(
            resolved.workspace.name,
            workspace_id.to_string(),
            resolved.root.id,
            resolved.root.path,
        );
        save_design_document(&path, &document).await?;
        Ok((path, document))
    }
}

fn new_design_document(
    title: String,
    workspace_id: String,
    root_id: String,
    workspace_root: String,
) -> RoderDesignDocument {
    let now = now_iso();
    let frame_id = "frame-root".to_string();
    let mut nodes = BTreeMap::new();
    nodes.insert(
        frame_id.clone(),
        RoderDesignNode {
            id: frame_id.clone(),
            node_type: "frame".to_string(),
            name: "Frame".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "#ffffff" })),
            stroke: None,
            extra: BTreeMap::new(),
        },
    );
    RoderDesignDocument {
        version: DESIGN_VERSION.to_string(),
        document_id: stable_id("design", &workspace_root),
        title,
        created_at: now.clone(),
        updated_at: now,
        nodes,
        root_ids: vec![frame_id],
        variables: BTreeMap::new(),
        assets: BTreeMap::new(),
        metadata: RoderDesignMetadata {
            workspace_id: Some(workspace_id),
            root_id: Some(root_id),
            workspace_root: Some(workspace_root),
        },
    }
}

async fn save_design_document(
    path: &PathBuf,
    document: &RoderDesignDocument,
) -> Result<(), JsonRpcError> {
    let data = serde_json::to_vec_pretty(document)
        .map_err(|err| internal_error(format!("serialize .roderdesign: {err}")))?;
    let tmp_path = path.with_extension(format!("roderdesign.tmp.{}", std::process::id()));
    tokio::fs::write(&tmp_path, data)
        .await
        .map_err(|err| internal_error(format!("write .roderdesign temp: {err}")))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|err| internal_error(format!("replace .roderdesign: {err}")))?;
    Ok(())
}

fn batch_get_nodes(
    document: &RoderDesignDocument,
    params: &DesignBatchGetParams,
) -> Vec<RoderDesignNode> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    if params.node_ids.is_empty() && params.patterns.is_empty() {
        for id in &document.root_ids {
            push_node(
                document,
                id,
                params.read_depth.unwrap_or(1),
                &mut seen,
                &mut result,
            );
        }
        return result;
    }
    for id in &params.node_ids {
        push_node(
            document,
            id,
            params.read_depth.unwrap_or(1),
            &mut seen,
            &mut result,
        );
    }
    if !params.patterns.is_empty() {
        let start_ids = params
            .parent_id
            .as_ref()
            .map(|id| vec![id.clone()])
            .unwrap_or_else(|| document.root_ids.clone());
        let mut queue = VecDeque::new();
        for id in start_ids {
            queue.push_back((id, 0_u32));
        }
        let max_search_depth = params.search_depth.unwrap_or(u32::MAX);
        while let Some((id, depth)) = queue.pop_front() {
            if depth > max_search_depth {
                continue;
            }
            if let Some(node) = document.nodes.get(&id) {
                if params
                    .patterns
                    .iter()
                    .any(|pattern| pattern_matches(node, pattern))
                {
                    push_node(
                        document,
                        &id,
                        params.read_depth.unwrap_or(1),
                        &mut seen,
                        &mut result,
                    );
                }
                for child_id in &node.child_ids {
                    queue.push_back((child_id.clone(), depth.saturating_add(1)));
                }
            }
        }
    }
    result
}

fn push_node(
    document: &RoderDesignDocument,
    node_id: &str,
    depth: u32,
    seen: &mut HashSet<String>,
    result: &mut Vec<RoderDesignNode>,
) {
    if !seen.insert(node_id.to_string()) {
        return;
    }
    let Some(node) = document.nodes.get(node_id) else {
        return;
    };
    result.push(node.clone());
    if depth == 0 {
        return;
    }
    for child_id in &node.child_ids {
        push_node(document, child_id, depth - 1, seen, result);
    }
}

fn pattern_matches(node: &RoderDesignNode, pattern: &DesignNodeSearchPattern) -> bool {
    if let Some(expected_type) = &pattern.node_type {
        if &node.node_type != expected_type {
            return false;
        }
    }
    if let Some(name) = &pattern.name {
        if !node.name.to_lowercase().contains(&name.to_lowercase()) {
            return false;
        }
    }
    true
}

fn apply_operation(
    document: &mut RoderDesignDocument,
    operation: DesignPatchOperation,
) -> Result<(), JsonRpcError> {
    match operation {
        DesignPatchOperation::InsertNode {
            parent_id,
            index,
            mut node,
        } => {
            if document.nodes.contains_key(&node.id) {
                return Err(invalid_params("node id already exists"));
            }
            node.parent_id = parent_id.clone();
            if let Some(parent_id) = parent_id {
                let parent = document
                    .nodes
                    .get_mut(&parent_id)
                    .ok_or_else(|| invalid_params("unknown parentId"))?;
                let insert_at = index
                    .unwrap_or(parent.child_ids.len())
                    .min(parent.child_ids.len());
                parent.child_ids.insert(insert_at, node.id.clone());
            } else {
                let insert_at = index
                    .unwrap_or(document.root_ids.len())
                    .min(document.root_ids.len());
                document.root_ids.insert(insert_at, node.id.clone());
            }
            document.nodes.insert(node.id.clone(), node);
        }
        DesignPatchOperation::UpdateNode { node_id, patch } => {
            let current = document
                .nodes
                .get(&node_id)
                .ok_or_else(|| invalid_params("unknown nodeId"))?;
            let mut value = serde_json::to_value(current).unwrap();
            merge_json(&mut value, patch);
            let updated: RoderDesignNode = serde_json::from_value(value)
                .map_err(|err| invalid_params(format!("invalid node patch: {err}")))?;
            document.nodes.insert(node_id, updated);
        }
        DesignPatchOperation::DeleteNode { node_id, recursive } => {
            delete_node(document, &node_id, recursive)?;
        }
        DesignPatchOperation::SetVariables { variables, replace } => {
            if replace {
                document.variables = variables;
            } else {
                document.variables.extend(variables);
            }
        }
    }
    Ok(())
}

async fn export_directory(
    source_path: &PathBuf,
    output_dir: Option<&str>,
) -> Result<PathBuf, JsonRpcError> {
    let dir = output_dir.map(PathBuf::from).unwrap_or_else(|| {
        source_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".roder")
            .join("design-exports")
    });
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| internal_error(format!("create design export dir: {err}")))?;
    Ok(dir)
}

fn node_to_svg(document: &RoderDesignDocument, node: &RoderDesignNode) -> String {
    let width = node.width.max(1.0);
    let height = node.height.max(1.0);
    let mut body = String::new();
    render_node_svg(document, node, -node.x, -node.y, &mut body);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

fn render_node_svg(
    document: &RoderDesignDocument,
    node: &RoderDesignNode,
    offset_x: f64,
    offset_y: f64,
    out: &mut String,
) {
    let x = node.x + offset_x;
    let y = node.y + offset_y;
    let fill = paint_color(node.fill.as_ref()).unwrap_or_else(|| {
        if node.node_type == "text" {
            "#18181b".to_string()
        } else {
            "#ffffff".to_string()
        }
    });
    let stroke = paint_color(node.stroke.as_ref()).unwrap_or_else(|| "#d4d4d8".to_string());
    match node.node_type.as_str() {
        "ellipse" => out.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{}" stroke="{}"/>"#,
            x + node.width / 2.0,
            y + node.height / 2.0,
            node.width / 2.0,
            node.height / 2.0,
            escape_xml(&fill),
            escape_xml(&stroke)
        )),
        "line" => out.push_str(&format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"/>"#,
            x,
            y,
            x + node.width,
            y + node.height,
            escape_xml(&stroke),
            stroke_width(node.stroke.as_ref())
        )),
        "image" => {
            if let Some(src) = node.extra.get("src").and_then(serde_json::Value::as_str) {
                if !src.is_empty() {
                    out.push_str(&format!(
                        r#"<image x="{}" y="{}" width="{}" height="{}" href="{}" preserveAspectRatio="xMidYMid slice"/>"#,
                        x,
                        y,
                        node.width.max(1.0),
                        node.height.max(1.0),
                        escape_xml(src)
                    ));
                } else {
                    render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, out);
                }
            } else {
                render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, out);
            }
        }
        "text" => out.push_str(&format!(
            r#"<text x="{}" y="{}" fill="{}" font-family="system-ui, sans-serif" font-size="16">{}</text>"#,
            x,
            y + 18.0,
            escape_xml(&fill),
            escape_xml(node.extra.get("content").and_then(serde_json::Value::as_str).unwrap_or(&node.name))
        )),
        _ => render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, out),
    }
    for child_id in &node.child_ids {
        if let Some(child) = document.nodes.get(child_id) {
            render_node_svg(document, child, x, y, out);
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
        width.max(1.0),
        height.max(1.0),
        escape_xml(fill),
        escape_xml(stroke)
    ));
}

fn paint_color(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Object(map) => map
            .get("value")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

fn stroke_width(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(serde_json::Value::as_object)
        .and_then(|map| map.get("width"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(2.0)
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
    sanitized
        .trim_matches('-')
        .chars()
        .take(48)
        .collect::<String>()
        .if_empty("node")
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn snapshot_layout(document: &RoderDesignDocument) -> Vec<DesignLayoutNode> {
    let mut nodes = document
        .nodes
        .values()
        .map(|node| {
            let mut problems = Vec::new();
            if node.width <= 0.0 || node.height <= 0.0 {
                problems.push("non-positive size".to_string());
            }
            if let Some(parent_id) = &node.parent_id {
                if !document.nodes.contains_key(parent_id) {
                    problems.push("missing parent".to_string());
                }
            }
            DesignLayoutNode {
                id: node.id.clone(),
                node_type: node.node_type.clone(),
                name: node.name.clone(),
                x: node.x,
                y: node.y,
                width: node.width,
                height: node.height,
                parent_id: node.parent_id.clone(),
                child_ids: node.child_ids.clone(),
                problems,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    nodes
}

fn design_guidelines() -> DesignGuidelinesResult {
    DesignGuidelinesResult {
        categories: vec![
            DesignGuidelineCategory {
                name: "workflow".to_string(),
                description: "How Roder agents should work with .roderdesign documents.".to_string(),
                guidelines: vec![
                    "Call design/get_editor_state or design_read before editing.".to_string(),
                    "Use batch reads instead of many one-node reads.".to_string(),
                    "Apply edits with typed patch operations and keep operations scoped.".to_string(),
                    "Run snapshot_layout after structural changes to catch invalid sizes or parent links.".to_string(),
                ],
            },
            DesignGuidelineCategory {
                name: "layout".to_string(),
                description: "Default product layout guidance for early Design Canvas documents.".to_string(),
                guidelines: vec![
                    "Use frames as artboards or major containers.".to_string(),
                    "Name frames and important nodes with user-recognizable labels.".to_string(),
                    "Prefer literal dimensions and colors until workspace token extraction lands.".to_string(),
                    "Keep generated designs simple enough to map to real application components.".to_string(),
                ],
            },
        ],
    }
}

fn delete_node(
    document: &mut RoderDesignDocument,
    node_id: &str,
    recursive: bool,
) -> Result<(), JsonRpcError> {
    let node = document
        .nodes
        .get(node_id)
        .cloned()
        .ok_or_else(|| invalid_params("unknown nodeId"))?;
    if !recursive && !node.child_ids.is_empty() {
        return Err(invalid_params("node has children; pass recursive=true"));
    }
    for child_id in node.child_ids.clone() {
        delete_node(document, &child_id, true)?;
    }
    if let Some(parent_id) = node.parent_id {
        if let Some(parent) = document.nodes.get_mut(&parent_id) {
            parent.child_ids.retain(|id| id != node_id);
        }
    } else {
        document.root_ids.retain(|id| id != node_id);
    }
    document.nodes.remove(node_id);
    Ok(())
}

fn merge_json(target: &mut serde_json::Value, patch: serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(target), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json(target.entry(key).or_insert(serde_json::Value::Null), value);
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

fn design_schema() -> serde_json::Value {
    serde_json::json!({
        "format": ".roderdesign",
        "version": DESIGN_VERSION,
        "nodeTypes": ["frame", "group", "text", "rectangle", "ellipse", "line", "path", "image", "icon", "component", "instance", "prompt", "annotation", "artifactRef"],
        "tools": ["design/get_editor_state", "design/batch_get", "design/patch", "design/get_variables", "design/snapshot_layout", "design/get_guidelines"]
    })
}

const DESIGN_RULES: &str = "Use design/get_editor_state first, combine reads with design/batch_get, mutate only with typed design/patch operations, and run design/snapshot_layout after structural edits.";

fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}

fn internal_error(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32000,
        message: message.into(),
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_get_returns_top_level_nodes_by_default() {
        let doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let nodes = batch_get_nodes(
            &doc,
            &DesignBatchGetParams {
                workspace_id: "ws".to_string(),
                root_id: None,
                node_ids: Vec::new(),
                patterns: Vec::new(),
                parent_id: None,
                read_depth: None,
                search_depth: None,
            },
        );
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_type, "frame");
    }

    #[test]
    fn image_nodes_export_svg_image_when_src_is_set() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let mut extra = BTreeMap::new();
        extra.insert(
            "src".to_string(),
            serde_json::Value::String("https://example.com/cat.png".to_string()),
        );
        let image = RoderDesignNode {
            id: "image-1".to_string(),
            node_type: "image".to_string(),
            name: "Hero".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 180.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: None,
            stroke: None,
            extra,
        };
        doc.nodes.insert(image.id.clone(), image);
        let svg = node_to_svg(&doc, doc.nodes.get("image-1").unwrap());
        assert!(svg.contains("<image "));
        assert!(svg.contains(r#"href="https://example.com/cat.png""#));
        assert!(svg.contains(r#"preserveAspectRatio="xMidYMid slice""#));
    }

    #[test]
    fn line_nodes_export_svg_line() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let line = RoderDesignNode {
            id: "line-1".to_string(),
            node_type: "line".to_string(),
            name: "Divider".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: None,
            stroke: Some(serde_json::json!({ "kind": "color", "value": "#111827", "width": 3 })),
            extra: BTreeMap::new(),
        };
        doc.nodes.insert(line.id.clone(), line);
        let svg = node_to_svg(&doc, doc.nodes.get("line-1").unwrap());
        assert!(svg.contains("<line "));
        assert!(svg.contains(r#"x1="0""#));
        assert!(svg.contains(r#"x2="100""#));
        assert!(svg.contains(r##"stroke="#111827""##));
        assert!(svg.contains(r#"stroke-width="3""#));
    }

    #[test]
    fn patch_insert_and_delete_updates_parent_children() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let child = RoderDesignNode {
            id: "text-1".to_string(),
            node_type: "text".to_string(),
            name: "Title".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 10.0,
            width: 200.0,
            height: 40.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: None,
            stroke: None,
            extra: BTreeMap::new(),
        };
        apply_operation(
            &mut doc,
            DesignPatchOperation::InsertNode {
                parent_id: Some("frame-root".to_string()),
                index: None,
                node: child,
            },
        )
        .unwrap();
        assert_eq!(doc.nodes["frame-root"].child_ids, vec!["text-1"]);
        apply_operation(
            &mut doc,
            DesignPatchOperation::DeleteNode {
                node_id: "text-1".to_string(),
                recursive: false,
            },
        )
        .unwrap();
        assert!(doc.nodes["frame-root"].child_ids.is_empty());
    }
}
