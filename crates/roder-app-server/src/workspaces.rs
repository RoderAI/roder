use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use roder_protocol::{
    JsonRpcError, Workspace, WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceForgetParams,
    WorkspaceForgetResult, WorkspaceListResult, WorkspaceRoot, WorkspaceRootInput,
    WorkspaceUpdateParams, WorkspaceUpdateResult,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceRegistryState {
    #[serde(default)]
    workspaces: Vec<Workspace>,
}

#[derive(Debug)]
pub(crate) struct WorkspaceRegistry {
    path: PathBuf,
    state: RwLock<Option<WorkspaceRegistryState>>,
    mutation_lock: Mutex<()>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedWorkspaceRoot {
    pub workspace: Workspace,
    pub root: WorkspaceRoot,
}

impl WorkspaceRegistry {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: RwLock::new(None),
            mutation_lock: Mutex::new(()),
        }
    }

    pub(crate) async fn list(
        &self,
        runtime_workspace: Option<String>,
    ) -> Result<WorkspaceListResult, JsonRpcError> {
        let state = self.load(runtime_workspace).await?;
        Ok(WorkspaceListResult {
            workspaces: state.workspaces,
        })
    }

    pub(crate) async fn create(
        &self,
        runtime_workspace: Option<String>,
        params: WorkspaceCreateParams,
    ) -> Result<WorkspaceCreateResult, JsonRpcError> {
        let _guard = self.mutation_lock.lock().await;
        let mut state = self.load(runtime_workspace).await?;
        let roots = validate_roots(params.roots)?;
        if roots.is_empty() {
            return Err(invalid_params(
                "workspace/create requires at least one root",
            ));
        }
        let default_root_id = default_root_id(&roots, params.default_root_path.as_deref())?;
        let id = workspace_id_for_roots(&roots);
        let name = params
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| roots[0].name.clone());
        let workspace = Workspace {
            id: id.clone(),
            name,
            roots,
            default_root_id,
            updated_at: now_ms(),
        };
        if let Some(existing) = state
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
        {
            *existing = workspace.clone();
        } else {
            state.workspaces.push(workspace.clone());
        }
        self.save(state).await?;
        Ok(WorkspaceCreateResult { workspace })
    }

    pub(crate) async fn update(
        &self,
        runtime_workspace: Option<String>,
        params: WorkspaceUpdateParams,
    ) -> Result<WorkspaceUpdateResult, JsonRpcError> {
        let _guard = self.mutation_lock.lock().await;
        let mut state = self.load(runtime_workspace).await?;
        let index = state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == params.workspace_id)
            .ok_or_else(|| invalid_params("unknown workspaceId"))?;
        let current = state.workspaces[index].clone();
        let roots = match params.roots {
            Some(roots) => {
                let roots = validate_roots(roots)?;
                if roots.is_empty() {
                    return Err(invalid_params(
                        "workspace/update requires at least one root",
                    ));
                }
                roots
            }
            None => current.roots,
        };
        let default_root_id = match params.default_root_id {
            Some(default_root_id) => {
                if !roots.iter().any(|root| root.id == default_root_id) {
                    return Err(invalid_params(
                        "defaultRootId must reference a workspace root",
                    ));
                }
                default_root_id
            }
            None if roots.iter().any(|root| root.id == current.default_root_id) => {
                current.default_root_id
            }
            None => roots[0].id.clone(),
        };
        let workspace = Workspace {
            id: params.workspace_id,
            name: params
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(current.name),
            roots,
            default_root_id,
            updated_at: now_ms(),
        };
        state.workspaces[index] = workspace.clone();
        self.save(state).await?;
        Ok(WorkspaceUpdateResult { workspace })
    }

    pub(crate) async fn forget(
        &self,
        runtime_workspace: Option<String>,
        params: WorkspaceForgetParams,
    ) -> Result<WorkspaceForgetResult, JsonRpcError> {
        let _guard = self.mutation_lock.lock().await;
        let mut state = self.load(runtime_workspace).await?;
        let before = state.workspaces.len();
        state
            .workspaces
            .retain(|workspace| workspace.id != params.workspace_id);
        let forgotten = state.workspaces.len() != before;
        self.save(state).await?;
        Ok(WorkspaceForgetResult {
            workspace_id: params.workspace_id,
            forgotten,
        })
    }

    pub(crate) async fn resolve_root(
        &self,
        runtime_workspace: Option<String>,
        workspace_id: &str,
        root_id: Option<&str>,
    ) -> Result<ResolvedWorkspaceRoot, JsonRpcError> {
        let state = self.load(runtime_workspace).await?;
        let workspace = state
            .workspaces
            .into_iter()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| invalid_params("unknown workspaceId"))?;
        let selected_root_id = root_id.unwrap_or(&workspace.default_root_id);
        let root = workspace
            .roots
            .iter()
            .find(|root| root.id == selected_root_id)
            .cloned()
            .ok_or_else(|| invalid_params("unknown rootId"))?;
        Ok(ResolvedWorkspaceRoot { workspace, root })
    }

    pub(crate) async fn resolve_workspace(
        &self,
        runtime_workspace: Option<String>,
        workspace_id: &str,
    ) -> Result<Workspace, JsonRpcError> {
        let state = self.load(runtime_workspace).await?;
        state
            .workspaces
            .into_iter()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| invalid_params("unknown workspaceId"))
    }

    async fn load(
        &self,
        runtime_workspace: Option<String>,
    ) -> Result<WorkspaceRegistryState, JsonRpcError> {
        {
            let state = self.state.read().await;
            if let Some(state) = state.as_ref() {
                return Ok(seed_runtime_workspace(state.clone(), runtime_workspace)?);
            }
        }
        let mut state = if self.path.exists() {
            let data = tokio::fs::read(&self.path)
                .await
                .map_err(|err| internal_error(format!("read workspace registry: {err}")))?;
            serde_json::from_slice(&data)
                .map_err(|err| internal_error(format!("parse workspace registry: {err}")))?
        } else {
            WorkspaceRegistryState::default()
        };
        state = seed_runtime_workspace(state, runtime_workspace)?;
        *self.state.write().await = Some(state.clone());
        Ok(state)
    }

    async fn save(&self, state: WorkspaceRegistryState) -> Result<(), JsonRpcError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| internal_error(format!("create workspace registry dir: {err}")))?;
        }
        let data = serde_json::to_vec_pretty(&state)
            .map_err(|err| internal_error(format!("serialize workspace registry: {err}")))?;
        let tmp_path = self.path.with_extension(format!(
            "json.tmp.{}.{}",
            std::process::id(),
            SAVE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        tokio::fs::write(&tmp_path, data)
            .await
            .map_err(|err| internal_error(format!("write workspace registry temp file: {err}")))?;
        tokio::fs::rename(&tmp_path, &self.path)
            .await
            .map_err(|err| internal_error(format!("replace workspace registry: {err}")))?;
        *self.state.write().await = Some(state);
        Ok(())
    }
}

pub(crate) fn validate_cwd(
    root: &WorkspaceRoot,
    cwd: Option<String>,
) -> Result<String, JsonRpcError> {
    let Some(cwd) = cwd else {
        return Ok(root.path.clone());
    };
    let cwd = canonical_dir(&cwd, "cwd")?;
    let root_path = PathBuf::from(&root.path);
    if cwd != root_path && !cwd.starts_with(&root_path) {
        return Err(invalid_params(
            "cwd must be the selected root or a child path",
        ));
    }
    Ok(cwd.display().to_string())
}

fn seed_runtime_workspace(
    mut state: WorkspaceRegistryState,
    runtime_workspace: Option<String>,
) -> Result<WorkspaceRegistryState, JsonRpcError> {
    let Some(runtime_workspace) = runtime_workspace else {
        return Ok(state);
    };
    let root_path = canonical_dir(&runtime_workspace, "runtime workspace")?;
    let root = workspace_root(root_path, None);
    if state.workspaces.iter().any(|workspace| {
        workspace
            .roots
            .iter()
            .any(|candidate| candidate.id == root.id)
    }) {
        return Ok(state);
    }
    state.workspaces.push(Workspace {
        id: workspace_id_for_roots(std::slice::from_ref(&root)),
        name: root.name.clone(),
        default_root_id: root.id.clone(),
        roots: vec![root],
        updated_at: now_ms(),
    });
    Ok(state)
}

fn validate_roots(inputs: Vec<WorkspaceRootInput>) -> Result<Vec<WorkspaceRoot>, JsonRpcError> {
    let mut roots = Vec::new();
    for input in inputs {
        let root = workspace_root(canonical_dir(&input.path, "workspace root")?, input.name);
        if !roots
            .iter()
            .any(|candidate: &WorkspaceRoot| candidate.id == root.id)
        {
            roots.push(root);
        }
    }
    Ok(roots)
}

fn default_root_id(
    roots: &[WorkspaceRoot],
    default_root_path: Option<&str>,
) -> Result<String, JsonRpcError> {
    let Some(default_root_path) = default_root_path else {
        return Ok(roots[0].id.clone());
    };
    let default_root = workspace_root(canonical_dir(default_root_path, "default root")?, None);
    roots
        .iter()
        .find(|root| root.id == default_root.id)
        .map(|root| root.id.clone())
        .ok_or_else(|| invalid_params("defaultRootPath must match a workspace root"))
}

fn workspace_root(path: PathBuf, name: Option<String>) -> WorkspaceRoot {
    let path_text = path.display().to_string();
    WorkspaceRoot {
        id: stable_id("root", &path_text),
        name: name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| path_name(&path)),
        path: path_text,
    }
}

fn workspace_id_for_roots(roots: &[WorkspaceRoot]) -> String {
    let mut keys = roots
        .iter()
        .map(|root| root.path.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    stable_id("ws", &keys.join("\n"))
}

fn canonical_dir(path: &str, label: &str) -> Result<PathBuf, JsonRpcError> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid_params(format!("{label} must be absolute")));
    }
    let path = path
        .canonicalize()
        .map_err(|err| invalid_params(format!("{label} is not accessible: {err}")))?;
    if !path.is_dir() {
        return Err(invalid_params(format!("{label} must be a directory")));
    }
    Ok(path)
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("workspace")
        .to_string()
}

fn stable_id(prefix: &str, value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}_{hash:016x}")
}

fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

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

    #[tokio::test]
    async fn registry_allows_registered_sibling_root() {
        let root = temp_root("registered-sibling-root");
        let runtime = root.join("runtime");
        let sibling = root.join("sibling");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let registry = WorkspaceRegistry::new(root.join("workspaces.json"));

        let created = registry
            .create(
                Some(runtime.display().to_string()),
                WorkspaceCreateParams {
                    name: None,
                    roots: vec![WorkspaceRootInput {
                        path: sibling.display().to_string(),
                        name: None,
                    }],
                    default_root_path: None,
                },
            )
            .await
            .unwrap();
        let resolved = registry
            .resolve_root(
                Some(runtime.display().to_string()),
                &created.workspace.id,
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            resolved.root.path,
            sibling.canonicalize().unwrap().display().to_string()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn registry_rejects_unknown_workspace_and_root_ids() {
        let root = temp_root("unknown-workspace-root");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let registry = WorkspaceRegistry::new(root.join("workspaces.json"));
        let created = registry
            .create(
                None,
                WorkspaceCreateParams {
                    name: None,
                    roots: vec![WorkspaceRootInput {
                        path: project.display().to_string(),
                        name: None,
                    }],
                    default_root_path: None,
                },
            )
            .await
            .unwrap();

        let unknown_workspace = registry
            .resolve_root(None, "ws_missing", None)
            .await
            .unwrap_err();
        assert_eq!(unknown_workspace.code, -32602);
        assert!(unknown_workspace.message.contains("unknown workspaceId"));

        let unknown_root = registry
            .resolve_root(None, &created.workspace.id, Some("root_missing"))
            .await
            .unwrap_err();
        assert_eq!(unknown_root.code, -32602);
        assert!(unknown_root.message.contains("unknown rootId"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn validate_cwd_defaults_to_root_and_rejects_sibling() {
        let temp = temp_root("validate-cwd");
        let workspace = temp.join("workspace");
        let child = workspace.join("child");
        let sibling = temp.join("sibling");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let root = workspace_root(workspace.canonicalize().unwrap(), None);

        assert_eq!(validate_cwd(&root, None).unwrap(), root.path.clone());
        assert_eq!(
            validate_cwd(&root, Some(child.display().to_string())).unwrap(),
            child.canonicalize().unwrap().display().to_string()
        );
        let error = validate_cwd(&root, Some(sibling.display().to_string())).unwrap_err();
        assert_eq!(error.code, -32602);
        assert!(error.message.contains("selected root or a child path"));
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn concurrent_creates_preserve_all_workspaces() {
        let root = temp_root("concurrent-creates");
        let left = root.join("left");
        let right = root.join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        let registry = std::sync::Arc::new(WorkspaceRegistry::new(root.join("workspaces.json")));

        let left_registry = std::sync::Arc::clone(&registry);
        let left_path = left.display().to_string();
        let left_task = tokio::spawn(async move {
            left_registry
                .create(
                    None,
                    WorkspaceCreateParams {
                        name: Some("left".to_string()),
                        roots: vec![WorkspaceRootInput {
                            path: left_path,
                            name: None,
                        }],
                        default_root_path: None,
                    },
                )
                .await
                .unwrap();
        });
        let right_registry = std::sync::Arc::clone(&registry);
        let right_path = right.display().to_string();
        let right_task = tokio::spawn(async move {
            right_registry
                .create(
                    None,
                    WorkspaceCreateParams {
                        name: Some("right".to_string()),
                        roots: vec![WorkspaceRootInput {
                            path: right_path,
                            name: None,
                        }],
                        default_root_path: None,
                    },
                )
                .await
                .unwrap();
        });

        left_task.await.unwrap();
        right_task.await.unwrap();
        let listed = registry.list(None).await.unwrap();
        assert_eq!(listed.workspaces.len(), 2);
        let names = listed
            .workspaces
            .iter()
            .map(|workspace| workspace.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names, ["left", "right"].into_iter().collect());
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "roder-workspaces-{prefix}-{}",
            uuid::Uuid::new_v4()
        ))
    }
}
