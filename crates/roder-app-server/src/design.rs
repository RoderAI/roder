use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;

use base64::Engine as _;
use roder_protocol::{
    DesignBatchGetParams, DesignBatchGetResult, DesignDocumentResult, DesignEditorStateResult,
    DesignExportNodesParams, DesignExportNodesResult, DesignExportedNode,
    DesignGetEditorStateParams, DesignGetGuidelinesParams, DesignGetScreenshotParams,
    DesignGetVariablesParams, DesignGuidelineCategory, DesignGuidelinesResult, DesignLayoutNode,
    DesignNodeAlias, DesignNodeSearchPattern, DesignPatchOperation, DesignPatchParams,
    DesignPatchResult, DesignScreenshotResult, DesignSetSelectionParams, DesignSetVariablesParams,
    DesignSnapshotLayoutParams, DesignSnapshotLayoutResult, DesignSpawnAgentsParams,
    DesignSpawnAgentsResult, DesignSpawnedAgentScope, DesignVariablesResult, DesignWorkspaceParams,
    JsonRpcError, RoderDesignDocument, RoderDesignMetadata, RoderDesignNode,
};
use time::OffsetDateTime;

use crate::AppServer;

const DESIGN_DIR_NAME: &str = "design";
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
            selected_node_ids: document.metadata.selected_node_ids.clone(),
            node_aliases: design_node_aliases(&document),
            document,
            schema: params.include_schema.then(design_schema),
            rules: params.include_schema.then(|| DESIGN_RULES.to_string()),
        })
        .unwrap())
    }

    pub(crate) async fn handle_design_set_selection(
        &self,
        params: DesignSetSelectionParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, mut document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let selected_node_ids = params
            .selected_node_ids
            .iter()
            .map(|node_id| {
                resolve_node_alias(&document, node_id).unwrap_or_else(|| node_id.clone())
            })
            .collect::<Vec<_>>();
        for node_id in &selected_node_ids {
            if !document.nodes.contains_key(node_id) {
                return Err(invalid_params(format!(
                    "unknown selected nodeId: {node_id}"
                )));
            }
        }
        document.metadata.selected_node_ids = selected_node_ids;
        document.updated_at = now_iso();
        save_design_document(&path, &document).await?;
        let _ = self
            .protocol_notifications
            .send(roder_protocol::JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "design/selectionChanged".to_string(),
                params: serde_json::json!({
                    "path": path.display().to_string(),
                    "workspaceId": params.workspace_id,
                    "rootId": params.root_id,
                    "documentId": document.document_id,
                    "selectedNodeIds": document.metadata.selected_node_ids,
                }),
            });
        Ok(serde_json::to_value(DesignEditorStateResult {
            path: path.display().to_string(),
            selected_node_ids: document.metadata.selected_node_ids.clone(),
            node_aliases: design_node_aliases(&document),
            document,
            schema: None,
            rules: None,
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
        let mut params = params;
        resolve_batch_get_aliases(&document, &mut params);
        let nodes = batch_get_nodes(&document, &params);
        Ok(serde_json::to_value(DesignBatchGetResult {
            path: path.display().to_string(),
            nodes,
            node_aliases: design_node_aliases(&document),
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

    pub(crate) async fn handle_design_set_variables(
        &self,
        params: DesignSetVariablesParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, mut document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        if params.replace {
            document.variables.clear();
        }
        for (key, value) in params.variables {
            document.variables.insert(key, value);
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
            applied: 1,
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

    pub(crate) async fn handle_design_spawn_agents(
        &self,
        params: DesignSpawnAgentsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let mut params = params;
        resolve_spawn_agent_aliases(&document, &mut params);
        let planned = spawn_agent_scopes(&document, &params)?;
        Ok(serde_json::to_value(DesignSpawnAgentsResult {
            path: path.display().to_string(),
            planned,
            allow_patch: params.allow_patch,
            allow_export: params.allow_export,
            require_review: params.require_review,
            instructions: "Launch one Roder subagent per planned design scope. Each subagent must read design state first, stay within its scope node, and respect allowPatch/allowExport/requireReview permissions.".to_string(),
        })
        .unwrap())
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
            let node_id = resolve_node_alias(&document, &node_id).unwrap_or(node_id);
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
        let _ = self
            .protocol_notifications
            .send(roder_protocol::JsonRpcNotification {
                jsonrpc: "2.0".to_string(),
                method: "design/exportCompleted".to_string(),
                params: serde_json::json!({
                    "path": path.display().to_string(),
                    "workspaceId": params.workspace_id,
                    "rootId": params.root_id,
                    "documentId": document.document_id,
                    "exported": exported,
                }),
            });
        Ok(serde_json::to_value(DesignExportNodesResult { exported }).unwrap())
    }

    pub(crate) async fn handle_design_get_screenshot(
        &self,
        params: DesignGetScreenshotParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params
            .format
            .as_deref()
            .is_some_and(|format| format != "svg")
        {
            return Err(invalid_params(
                "only svg screenshot fallback is supported in this phase",
            ));
        }
        let (path, document) = self
            .load_or_create_design_document(&params.workspace_id, params.root_id.as_deref())
            .await?;
        let node_id = params.node_id.as_deref().map(|node_id| {
            resolve_node_alias(&document, node_id).unwrap_or_else(|| node_id.to_string())
        });
        let svg = if let Some(node_id) = &node_id {
            let node = document
                .nodes
                .get(node_id)
                .ok_or_else(|| invalid_params(format!("unknown nodeId: {node_id}")))?;
            node_to_svg(&document, node)
        } else {
            document_to_svg(&document)
        };
        let data_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
        );
        Ok(serde_json::to_value(DesignScreenshotResult {
            path: path.display().to_string(),
            node_id,
            mime_type: "image/svg+xml".to_string(),
            data_url,
        })
        .unwrap())
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
            let operation = resolve_operation_aliases(&document, operation);
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
        let path = design_document_path(&resolved.workspace.name, workspace_id, &resolved.root.id)?;
        if path.exists() {
            let data = tokio::fs::read(&path)
                .await
                .map_err(|err| internal_error(format!("read project.roderdesign: {err}")))?;
            let document = serde_json::from_slice(&data)
                .map_err(|err| invalid_params(format!("parse project.roderdesign: {err}")))?;
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
            selected_node_ids: Vec::new(),
        },
    }
}

fn design_document_path(
    project_name: &str,
    workspace_id: &str,
    root_id: &str,
) -> Result<PathBuf, JsonRpcError> {
    let home = dirs::home_dir()
        .ok_or_else(|| internal_error("resolve home directory for design document"))?;
    Ok(home
        .join(".roder")
        .join(DESIGN_DIR_NAME)
        .join(design_document_file_name(
            project_name,
            workspace_id,
            root_id,
        )))
}

fn design_document_file_name(project_name: &str, workspace_id: &str, root_id: &str) -> String {
    let slug = slugify_project_name(project_name);
    let stable = stable_id("project", &format!("{workspace_id}:{root_id}"));
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

async fn save_design_document(
    path: &PathBuf,
    document: &RoderDesignDocument,
) -> Result<(), JsonRpcError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| internal_error(format!("create design directory: {err}")))?;
    }
    let data = serde_json::to_vec_pretty(document)
        .map_err(|err| internal_error(format!("serialize project.roderdesign: {err}")))?;
    let tmp_path = path.with_extension(format!("roderdesign.tmp.{}", std::process::id()));
    tokio::fs::write(&tmp_path, data)
        .await
        .map_err(|err| internal_error(format!("write project.roderdesign temp: {err}")))?;
    tokio::fs::rename(&tmp_path, path)
        .await
        .map_err(|err| internal_error(format!("replace project.roderdesign: {err}")))?;
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

fn design_node_aliases(document: &RoderDesignDocument) -> Vec<DesignNodeAlias> {
    document
        .nodes
        .values()
        .enumerate()
        .map(|(index, node)| DesignNodeAlias {
            alias: format!("n{}", index + 1),
            node_id: node.id.clone(),
            name: node.name.clone(),
            node_type: node.node_type.clone(),
        })
        .collect()
}

fn resolve_node_alias(document: &RoderDesignDocument, id_or_alias: &str) -> Option<String> {
    if document.nodes.contains_key(id_or_alias) {
        return Some(id_or_alias.to_string());
    }
    let alias = id_or_alias.strip_prefix('n')?;
    let index = alias.parse::<usize>().ok()?.checked_sub(1)?;
    document
        .nodes
        .values()
        .nth(index)
        .map(|node| node.id.clone())
}

fn resolve_batch_get_aliases(document: &RoderDesignDocument, params: &mut DesignBatchGetParams) {
    for node_id in &mut params.node_ids {
        if let Some(resolved) = resolve_node_alias(document, node_id) {
            *node_id = resolved;
        }
    }
    if let Some(parent_id) = &mut params.parent_id {
        if let Some(resolved) = resolve_node_alias(document, parent_id) {
            *parent_id = resolved;
        }
    }
}

fn resolve_spawn_agent_aliases(
    document: &RoderDesignDocument,
    params: &mut DesignSpawnAgentsParams,
) {
    for node_id in &mut params.scope_node_ids {
        if let Some(resolved) = resolve_node_alias(document, node_id) {
            *node_id = resolved;
        }
    }
}

fn resolve_operation_aliases(
    document: &RoderDesignDocument,
    operation: DesignPatchOperation,
) -> DesignPatchOperation {
    match operation {
        DesignPatchOperation::InsertNode {
            parent_id,
            index,
            node,
        } => DesignPatchOperation::InsertNode {
            parent_id: parent_id.map(|id| resolve_node_alias(document, &id).unwrap_or(id)),
            index,
            node,
        },
        DesignPatchOperation::UpdateNode { node_id, patch } => DesignPatchOperation::UpdateNode {
            node_id: resolve_node_alias(document, &node_id).unwrap_or(node_id),
            patch,
        },
        DesignPatchOperation::DeleteNode { node_id, recursive } => {
            DesignPatchOperation::DeleteNode {
                node_id: resolve_node_alias(document, &node_id).unwrap_or(node_id),
                recursive,
            }
        }
        DesignPatchOperation::ReorderNode { node_id, index } => DesignPatchOperation::ReorderNode {
            node_id: resolve_node_alias(document, &node_id).unwrap_or(node_id),
            index,
        },
        DesignPatchOperation::SetVariables { variables, replace } => {
            DesignPatchOperation::SetVariables { variables, replace }
        }
    }
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
        DesignPatchOperation::ReorderNode { node_id, index } => {
            reorder_node(document, &node_id, index)?;
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

fn reorder_node(
    document: &mut RoderDesignDocument,
    node_id: &str,
    index: usize,
) -> Result<(), JsonRpcError> {
    let parent_id = document
        .nodes
        .get(node_id)
        .ok_or_else(|| invalid_params("unknown nodeId"))?
        .parent_id
        .clone();
    let siblings = if let Some(parent_id) = parent_id {
        &mut document
            .nodes
            .get_mut(&parent_id)
            .ok_or_else(|| invalid_params("unknown parentId"))?
            .child_ids
    } else {
        &mut document.root_ids
    };
    let current_index = siblings
        .iter()
        .position(|id| id == node_id)
        .ok_or_else(|| invalid_params("node is not listed in its parent/root order"))?;
    let node_id = siblings.remove(current_index);
    let insert_at = index.min(siblings.len());
    siblings.insert(insert_at, node_id);
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

fn document_to_svg(document: &RoderDesignDocument) -> String {
    let bounds = document_bounds(document).unwrap_or((0.0, 0.0, 800.0, 600.0));
    let (min_x, min_y, width, height) = bounds;
    let mut body = String::new();
    for root_id in &document.root_ids {
        if let Some(node) = document.nodes.get(root_id) {
            render_node_svg(document, node, -min_x, -min_y, &mut body);
        }
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">{}</svg>"#,
        width.max(1.0),
        height.max(1.0),
        width.max(1.0),
        height.max(1.0),
        body
    )
}

fn document_bounds(document: &RoderDesignDocument) -> Option<(f64, f64, f64, f64)> {
    let mut bounds = document
        .root_ids
        .iter()
        .filter_map(|id| document.nodes.get(id))
        .map(|node| {
            (
                node.x,
                node.y,
                node.x + node.width.max(1.0),
                node.y + node.height.max(1.0),
            )
        });
    let first = bounds.next()?;
    let (min_x, min_y, max_x, max_y) = bounds.fold(first, |acc, next| {
        (
            acc.0.min(next.0),
            acc.1.min(next.1),
            acc.2.max(next.2),
            acc.3.max(next.3),
        )
    });
    Some((min_x, min_y, max_x - min_x, max_y - min_y))
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
    let opacity = opacity_attr(node.opacity);
    let transform = transform_attr(node.rotation, x + node.width / 2.0, y + node.height / 2.0);
    let corner_radius = corner_radius(node.extra.get("cornerRadius"));
    let fill = paint_color(node.fill.as_ref()).unwrap_or_else(|| {
        if node.node_type == "text" {
            "#18181b".to_string()
        } else {
            "#ffffff".to_string()
        }
    });
    let stroke = paint_color(node.stroke.as_ref()).unwrap_or_else(|| "#d4d4d8".to_string());
    if node.node_type == "component" || node.node_type == "instance" {
        let source_component_id = node
            .extra
            .get("sourceComponentId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let overrides = node
            .extra
            .get("overrides")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            r#"<!-- roder-design-node type="{}" id="{}" component-id="{}" source-component-id="{}" overrides="{}" -->"#,
            escape_xml(&node.node_type),
            escape_xml(&node.id),
            escape_xml(
                node.extra
                    .get("componentId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&node.id)
            ),
            escape_xml(source_component_id),
            escape_xml(&overrides)
        ));
    }
    match node.node_type.as_str() {
        "ellipse" => out.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{}" stroke="{}"{}{} />"#,
            x + node.width / 2.0,
            y + node.height / 2.0,
            node.width / 2.0,
            node.height / 2.0,
            escape_xml(&fill),
            escape_xml(&stroke),
            opacity,
            transform
        )),
        "line" => out.push_str(&format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}" stroke-linecap="round"{}{} />"#,
            x,
            y,
            x + node.width,
            y + node.height,
            escape_xml(&stroke),
            stroke_width(node.stroke.as_ref()),
            opacity,
            transform
        )),
        "path" => {
            let view_box = node
                .extra
                .get("viewBox")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("0 0 {} {}", node.width.max(1.0), node.height.max(1.0)));
            let path_data = node
                .extra
                .get("pathData")
                .or_else(|| node.extra.get("d"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let path_fill = if fill == "transparent" { "none" } else { fill.as_str() };
            out.push_str(&format!(
                r#"<svg x="{}" y="{}" width="{}" height="{}" viewBox="{}"{}{}><path d="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round" /></svg>"#,
                x,
                y,
                node.width.max(1.0),
                node.height.max(1.0),
                escape_xml(&view_box),
                opacity,
                transform,
                escape_xml(path_data),
                escape_xml(path_fill),
                escape_xml(&stroke),
                stroke_width(node.stroke.as_ref())
            ));
        }
        "icon" => {
            let view_box = node
                .extra
                .get("viewBox")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("0 0 24 24");
            let path_data = node
                .extra
                .get("svg")
                .or_else(|| node.extra.get("pathData"))
                .or_else(|| node.extra.get("d"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            out.push_str(&format!(
                r#"<svg x="{}" y="{}" width="{}" height="{}" viewBox="{}"{}{}><path d="{}" fill="{}" /></svg>"#,
                x,
                y,
                node.width.max(1.0),
                node.height.max(1.0),
                escape_xml(view_box),
                opacity,
                transform,
                escape_xml(path_data),
                escape_xml(&fill)
            ));
        }
        "image" => {
            if let Some(src) = node.extra.get("src").and_then(serde_json::Value::as_str) {
                if !src.is_empty() {
                    out.push_str(&format!(
                        r#"<image x="{}" y="{}" width="{}" height="{}" href="{}" preserveAspectRatio="xMidYMid slice"{}{} />"#,
                        x,
                        y,
                        node.width.max(1.0),
                        node.height.max(1.0),
                        escape_xml(src),
                        opacity,
                        transform
                    ));
                } else {
                    render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, corner_radius, &opacity, &transform, out);
                }
            } else {
                render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, corner_radius, &opacity, &transform, out);
            }
        }
        "prompt" => {
            render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, corner_radius, &opacity, &transform, out);
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
                escape_xml(node.extra.get("prompt").or_else(|| node.extra.get("content")).and_then(serde_json::Value::as_str).unwrap_or("Describe the design change for Roder..."))
            ));
        }
        "text" => {
            let font_size = font_size(node.extra.get("fontSize"));
            let font_weight = font_weight(node.extra.get("fontWeight"));
            let anchor = text_anchor(node.extra.get("textAlign"));
            let text_x = match anchor {
                "middle" => x + node.width / 2.0,
                "end" => x + node.width,
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
            escape_xml(node.extra.get("content").and_then(serde_json::Value::as_str).unwrap_or(&node.name))
        ));
        }
        _ => render_placeholder_rect(x, y, node.width, node.height, &fill, &stroke, corner_radius, &opacity, &transform, out),
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
    corner_radius: f64,
    opacity: &str,
    transform: &str,
    out: &mut String,
) {
    out.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" ry="{}" fill="{}" stroke="{}"{}{} />"#,
        x,
        y,
        width.max(1.0),
        height.max(1.0),
        corner_radius,
        corner_radius,
        escape_xml(fill),
        escape_xml(stroke),
        opacity,
        transform
    ));
}

fn corner_radius(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(serde_json::Value::as_f64)
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

fn font_size(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(16.0)
        .round()
        .clamp(8.0, 144.0)
}

fn font_weight(value: Option<&serde_json::Value>) -> f64 {
    let weight = value.and_then(serde_json::Value::as_f64).unwrap_or(500.0);
    ((weight / 100.0).round() * 100.0).clamp(100.0, 900.0)
}

fn text_anchor(value: Option<&serde_json::Value>) -> &'static str {
    match value.and_then(serde_json::Value::as_str) {
        Some("center") => "middle",
        Some("right") => "end",
        _ => "start",
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

fn spawn_agent_scopes(
    document: &RoderDesignDocument,
    params: &DesignSpawnAgentsParams,
) -> Result<Vec<DesignSpawnedAgentScope>, JsonRpcError> {
    let scope_ids = if params.scope_node_ids.is_empty() {
        document.root_ids.clone()
    } else {
        params.scope_node_ids.clone()
    };
    let mut planned = Vec::new();
    let mut seen = HashSet::new();
    for scope_id in scope_ids {
        if !seen.insert(scope_id.clone()) {
            continue;
        }
        let node = document
            .nodes
            .get(&scope_id)
            .ok_or_else(|| invalid_params(format!("unknown scopeNodeId: {scope_id}")))?;
        if !can_spawn_agent_for_node(node) {
            return Err(invalid_params(format!(
                "scopeNodeId must reference a frame, group, component, or instance: {scope_id}"
            )));
        }
        let base_prompt = params
            .prompt
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty())
            .unwrap_or("Improve this design scope while preserving its intent.");
        planned.push(DesignSpawnedAgentScope {
            alias: resolve_node_alias(document, &node.id)
                .and_then(|_| {
                    design_node_aliases(document)
                        .into_iter()
                        .find(|alias| alias.node_id == node.id)
                        .map(|alias| alias.alias)
                })
                .unwrap_or_else(|| node.id.clone()),
            scope_node_id: node.id.clone(),
            scope_name: node.name.clone(),
            node_type: node.node_type.clone(),
            child_count: node.child_ids.len(),
            prompt: format!(
                "{base_prompt}\n\nScope: {} ({}, {})\nPermissions: allowPatch={}, allowExport={}, requireReview={}",
                node.name,
                node.node_type,
                node.id,
                params.allow_patch,
                params.allow_export,
                params.require_review
            ),
        });
    }
    Ok(planned)
}

fn can_spawn_agent_for_node(node: &RoderDesignNode) -> bool {
    matches!(
        node.node_type.as_str(),
        "frame" | "group" | "component" | "instance"
    )
}

fn design_guidelines() -> DesignGuidelinesResult {
    DesignGuidelinesResult {
        categories: vec![
            DesignGuidelineCategory {
                name: "workflow".to_string(),
                description: "How Roder agents should work with project-specific .roderdesign documents.".to_string(),
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
        "tools": ["design/get_editor_state", "design/batch_get", "design/patch", "design/get_variables", "design/set_variables", "design/set_selection", "design/snapshot_layout", "design/get_guidelines", "design/get_screenshot", "design/export_nodes", "design/spawn_agents"]
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
    fn node_aliases_resolve_for_reads_and_patches() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let aliases = design_node_aliases(&doc);
        assert_eq!(aliases[0].alias, "n1");
        assert_eq!(
            resolve_node_alias(&doc, "n1"),
            Some("frame-root".to_string())
        );

        let operation = resolve_operation_aliases(
            &doc,
            DesignPatchOperation::UpdateNode {
                node_id: "n1".to_string(),
                patch: serde_json::json!({ "name": "Hero" }),
            },
        );
        apply_operation(&mut doc, operation).unwrap();
        assert_eq!(doc.nodes.get("frame-root").unwrap().name, "Hero");
    }

    #[test]
    fn set_variables_patch_can_merge_and_replace_tokens() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let mut initial = BTreeMap::new();
        initial.insert(
            "color.primary".to_string(),
            serde_json::json!({ "kind": "color", "value": "#2563eb" }),
        );
        apply_operation(
            &mut doc,
            DesignPatchOperation::SetVariables {
                variables: initial,
                replace: false,
            },
        )
        .unwrap();
        assert!(doc.variables.contains_key("color.primary"));

        let mut replacement = BTreeMap::new();
        replacement.insert(
            "space.4".to_string(),
            serde_json::json!({ "kind": "spacing", "value": 16 }),
        );
        apply_operation(
            &mut doc,
            DesignPatchOperation::SetVariables {
                variables: replacement,
                replace: true,
            },
        )
        .unwrap();
        assert!(!doc.variables.contains_key("color.primary"));
        assert!(doc.variables.contains_key("space.4"));
    }

    #[test]
    fn design_document_path_uses_home_roder_design_project_file() {
        let path = design_document_path("Gode Desktop", "workspace-123", "root-456").unwrap();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap();
        assert!(file_name.starts_with("gode-desktop-project_"));
        assert!(file_name.ends_with(".roderdesign"));
        assert!(
            path.parent()
                .is_some_and(|parent| parent.ends_with(".roder/design"))
        );
    }

    #[test]
    fn editor_state_carries_selected_node_ids() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        doc.metadata.selected_node_ids = vec!["frame-root".to_string()];
        let value = serde_json::to_value(DesignEditorStateResult {
            path: "/tmp/home/.roder/design/test-project_123.roderdesign".to_string(),
            selected_node_ids: doc.metadata.selected_node_ids.clone(),
            node_aliases: design_node_aliases(&doc),
            document: doc,
            schema: None,
            rules: None,
        })
        .unwrap();
        assert_eq!(
            value
                .get("selectedNodeIds")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(1)
        );
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
            rotation: Some(15.0),
            opacity: Some(0.5),
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
        assert!(svg.contains(r#"opacity="0.5""#));
        assert!(svg.contains(r#"transform="rotate(15 "#));
        assert!(svg.contains(r#"preserveAspectRatio="xMidYMid slice""#));
    }

    #[test]
    fn document_screenshot_svg_fallback_can_encode_data_url() {
        let doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let svg = document_to_svg(&doc);
        assert!(svg.contains("<svg"));
        assert!(svg.contains(r#"width="800""#));
        assert!(svg.contains(r#"viewBox="0 0 800 600""#));
        assert!(svg.contains(r#"<rect x="0" y="0" width="800" height="600""#));
        let data_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
        );
        assert!(data_url.starts_with("data:image/svg+xml;base64,"));
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
            rotation: Some(90.0),
            opacity: Some(0.25),
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
        assert!(svg.contains(r#"opacity="0.25""#));
        assert!(svg.contains(r#"transform="rotate(90 "#));
    }

    #[test]
    fn rectangle_nodes_export_corner_radius() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let rectangle = RoderDesignNode {
            id: "rect-1".to_string(),
            node_type: "rectangle".to_string(),
            name: "Card".to_string(),
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
            fill: Some(serde_json::json!({ "kind": "color", "value": "#ffffff" })),
            stroke: Some(serde_json::json!({ "kind": "color", "value": "#111827" })),
            extra: BTreeMap::from([("cornerRadius".to_string(), serde_json::json!(18))]),
        };
        doc.nodes.insert(rectangle.id.clone(), rectangle);
        let svg = node_to_svg(&doc, doc.nodes.get("rect-1").unwrap());
        assert!(svg.contains("<rect "));
        assert!(svg.contains(r#"rx="18""#));
        assert!(svg.contains(r#"ry="18""#));
    }

    #[test]
    fn path_and_icon_nodes_export_vector_svg() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let path = RoderDesignNode {
            id: "path-1".to_string(),
            node_type: "path".to_string(),
            name: "Curve".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 160.0,
            height: 80.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "transparent" })),
            stroke: Some(serde_json::json!({ "kind": "color", "value": "#111827", "width": 3 })),
            extra: BTreeMap::from([
                (
                    "pathData".to_string(),
                    serde_json::json!("M0 80 C40 0 120 0 160 80"),
                ),
                ("viewBox".to_string(), serde_json::json!("0 0 160 80")),
            ]),
        };
        let icon = RoderDesignNode {
            id: "icon-1".to_string(),
            node_type: "icon".to_string(),
            name: "Star".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 20.0,
            y: 30.0,
            width: 24.0,
            height: 24.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "#f59e0b" })),
            stroke: None,
            extra: BTreeMap::from([
                (
                    "svg".to_string(),
                    serde_json::json!("M12 2l3 7h7l-5.5 4.5L18 21l-6-4-6 4 1.5-7.5L2 9h7z"),
                ),
                ("viewBox".to_string(), serde_json::json!("0 0 24 24")),
            ]),
        };
        doc.nodes.insert(path.id.clone(), path);
        doc.nodes.insert(icon.id.clone(), icon);
        let path_svg = node_to_svg(&doc, doc.nodes.get("path-1").unwrap());
        let icon_svg = node_to_svg(&doc, doc.nodes.get("icon-1").unwrap());
        assert!(path_svg.contains(r#"viewBox="0 0 160 80""#));
        assert!(path_svg.contains(r#"d="M0 80 C40 0 120 0 160 80""#));
        assert!(path_svg.contains(r#"stroke-width="3""#));
        assert!(icon_svg.contains(r#"viewBox="0 0 24 24""#));
        assert!(icon_svg.contains(r##"fill="#f59e0b""##));
    }

    #[test]
    fn prompt_nodes_export_prompt_card() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let prompt = RoderDesignNode {
            id: "prompt-1".to_string(),
            node_type: "prompt".to_string(),
            name: "Prompt".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 220.0,
            height: 120.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "#fef3c7" })),
            stroke: Some(serde_json::json!({ "kind": "color", "value": "#f59e0b" })),
            extra: BTreeMap::from([(
                "prompt".to_string(),
                serde_json::json!("Make the hero card warmer & clearer"),
            )]),
        };
        doc.nodes.insert(prompt.id.clone(), prompt);
        let svg = node_to_svg(&doc, doc.nodes.get("prompt-1").unwrap());
        assert!(svg.contains("<rect "));
        assert!(svg.contains(">Prompt</text>"));
        assert!(svg.contains("Make the hero card warmer &amp; clearer"));
    }

    #[test]
    fn text_nodes_export_typography() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let text = RoderDesignNode {
            id: "text-1".to_string(),
            node_type: "text".to_string(),
            name: "Headline".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 80.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "#111827" })),
            stroke: None,
            extra: BTreeMap::from([
                ("content".to_string(), serde_json::json!("Hello")),
                ("fontSize".to_string(), serde_json::json!(32)),
                ("fontWeight".to_string(), serde_json::json!(700)),
                ("textAlign".to_string(), serde_json::json!("center")),
            ]),
        };
        doc.nodes.insert(text.id.clone(), text);
        let svg = node_to_svg(&doc, doc.nodes.get("text-1").unwrap());
        assert!(svg.contains("<text "));
        assert!(svg.contains(r#"font-size="32""#));
        assert!(svg.contains(r#"font-weight="700""#));
        assert!(svg.contains(r#"text-anchor="middle""#));
        assert!(svg.contains(r#"x="100""#));
        assert!(svg.contains("Hello"));
    }

    #[test]
    fn component_nodes_export_identity_metadata() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let component = RoderDesignNode {
            id: "instance-1".to_string(),
            node_type: "instance".to_string(),
            name: "Card Instance".to_string(),
            parent_id: None,
            child_ids: Vec::new(),
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 120.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: Some(serde_json::json!({ "kind": "color", "value": "#ffffff" })),
            stroke: None,
            extra: BTreeMap::from([
                (
                    "componentId".to_string(),
                    serde_json::json!("component:card"),
                ),
                (
                    "sourceComponentId".to_string(),
                    serde_json::json!("component-1"),
                ),
                (
                    "overrides".to_string(),
                    serde_json::json!(["fill", "width"]),
                ),
            ]),
        };
        doc.nodes.insert(component.id.clone(), component);
        let svg = node_to_svg(&doc, doc.nodes.get("instance-1").unwrap());
        assert!(svg.contains(r#"roder-design-node type="instance" id="instance-1" component-id="component:card" source-component-id="component-1" overrides="fill,width""#));
        assert!(svg.contains("<rect "));
    }

    #[test]
    fn spawn_agents_plans_container_scopes() {
        let doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let scopes = spawn_agent_scopes(
            &doc,
            &DesignSpawnAgentsParams {
                workspace_id: "ws".to_string(),
                root_id: Some("root".to_string()),
                scope_node_ids: vec!["frame-root".to_string()],
                prompt: Some("Polish this frame".to_string()),
                allow_patch: true,
                allow_export: true,
                require_review: false,
            },
        )
        .unwrap();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].scope_node_id, "frame-root");
        assert_eq!(scopes[0].node_type, "frame");
        assert!(scopes[0].prompt.contains("Polish this frame"));
        assert!(scopes[0].prompt.contains("allowPatch=true"));
    }

    #[test]
    fn spawn_agents_rejects_non_container_scope() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        doc.nodes.insert(
            "text-1".to_string(),
            RoderDesignNode {
                id: "text-1".to_string(),
                node_type: "text".to_string(),
                name: "Title".to_string(),
                parent_id: None,
                child_ids: Vec::new(),
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
                rotation: None,
                opacity: None,
                visible: Some(true),
                locked: None,
                fill: None,
                stroke: None,
                extra: BTreeMap::new(),
            },
        );
        let error = spawn_agent_scopes(
            &doc,
            &DesignSpawnAgentsParams {
                workspace_id: "ws".to_string(),
                root_id: None,
                scope_node_ids: vec!["text-1".to_string()],
                prompt: None,
                allow_patch: false,
                allow_export: false,
                require_review: true,
            },
        )
        .unwrap_err();
        assert!(
            error
                .message
                .contains("frame, group, component, or instance")
        );
    }

    #[test]
    fn patch_reorder_node_updates_root_and_child_order() {
        let mut doc = new_design_document(
            "Test".to_string(),
            "ws".to_string(),
            "root".to_string(),
            "/tmp/ws".to_string(),
        );
        let mut frame_b = doc.nodes["frame-root"].clone();
        frame_b.id = "frame-b".to_string();
        frame_b.name = "Frame B".to_string();
        frame_b.child_ids = Vec::new();
        doc.root_ids.push(frame_b.id.clone());
        doc.nodes.insert(frame_b.id.clone(), frame_b);
        apply_operation(
            &mut doc,
            DesignPatchOperation::ReorderNode {
                node_id: "frame-b".to_string(),
                index: 0,
            },
        )
        .unwrap();
        assert_eq!(doc.root_ids.first().map(String::as_str), Some("frame-b"));

        let child_a = RoderDesignNode {
            id: "child-a".to_string(),
            node_type: "text".to_string(),
            name: "Child A".to_string(),
            parent_id: Some("frame-b".to_string()),
            child_ids: Vec::new(),
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 20.0,
            rotation: None,
            opacity: None,
            visible: Some(true),
            locked: None,
            fill: None,
            stroke: None,
            extra: BTreeMap::new(),
        };
        let mut child_b = child_a.clone();
        child_b.id = "child-b".to_string();
        child_b.name = "Child B".to_string();
        doc.nodes.get_mut("frame-b").unwrap().child_ids =
            vec!["child-a".to_string(), "child-b".to_string()];
        doc.nodes.insert("child-a".to_string(), child_a);
        doc.nodes.insert("child-b".to_string(), child_b);
        apply_operation(
            &mut doc,
            DesignPatchOperation::ReorderNode {
                node_id: "child-a".to_string(),
                index: 1,
            },
        )
        .unwrap();
        assert_eq!(
            doc.nodes["frame-b"].child_ids,
            vec!["child-b".to_string(), "child-a".to_string()]
        );
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
