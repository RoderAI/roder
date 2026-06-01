use std::path::PathBuf;
use std::sync::Arc;

use roder_api::version_control::{
    RegistryVcsProviderResolver, VcsChangedContentPage, VcsChangedFile, VcsError, VcsLineOfWork,
    VcsLineSwitchRequest, VcsListChangesRequest, VcsOperation, VcsOperationResult, VcsProvider,
    VcsProviderResolution, VcsProviderResolver, VcsReadChangedContentRequest, VcsResolveRequest,
    VcsRestoreRequest, VcsSelectionRequest, VcsSnapshot, VcsSnapshotCreateRequest, VcsStatus,
    VcsStatusRequest, VcsSyncOperation, VcsSyncRequest,
};
use roder_protocol::{
    JsonRpcError, VcsChangesListParams, VcsChangesListResult, VcsChangesReadParams,
    VcsChangesTotals, VcsLineSwitchParams, VcsRestoreParams, VcsSelectionParams,
    VcsSnapshotCreateParams, VcsSyncParams, VcsWorkspaceParams,
};

const DEFAULT_PATCH_LIMIT: usize = 400;
const DEFAULT_LIST_LIMIT: usize = 500;

pub(crate) async fn status(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsWorkspaceParams,
) -> Result<VcsStatus, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::Status,
    )
    .await?;
    provider
        .status(VcsStatusRequest {
            workspace_root: workspace,
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn list_changes(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsChangesListParams,
) -> Result<VcsChangesListResult, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::ChangesList,
    )
    .await?;
    let change_set = provider
        .status_with_changes(VcsListChangesRequest {
            workspace_root: workspace,
        })
        .await
        .map_err(vcs_error)?;
    let mut files = change_set.files;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    let totals = totals(&files);
    let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT).max(1);
    let truncated = files.len() > limit;
    files.truncate(limit);

    Ok(VcsChangesListResult {
        status: change_set.status,
        files,
        totals,
        truncated,
    })
}

pub(crate) async fn read_change(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsChangesReadParams,
) -> Result<VcsChangedContentPage, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::ChangesRead,
    )
    .await?;
    provider
        .read_changed_content(VcsReadChangedContentRequest {
            workspace_root: workspace,
            path: PathBuf::from(params.path),
            offset: params.offset.min(u32::MAX as usize) as u32,
            limit: params
                .limit
                .unwrap_or(DEFAULT_PATCH_LIMIT)
                .max(1)
                .min(u32::MAX as usize) as u32,
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn select(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsSelectionParams,
) -> Result<VcsOperationResult, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::Selection,
    )
    .await?;
    provider
        .select(VcsSelectionRequest {
            workspace_root: workspace,
            paths: paths(params.paths),
            granularity: params.granularity,
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn create_snapshot(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsSnapshotCreateParams,
) -> Result<VcsSnapshot, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::SnapshotCreate,
    )
    .await?;
    provider
        .create_snapshot(VcsSnapshotCreateRequest {
            workspace_root: workspace,
            message: params.message,
            paths: paths(params.paths),
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn restore(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsRestoreParams,
) -> Result<VcsOperationResult, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::Restore,
    )
    .await?;
    provider
        .restore(VcsRestoreRequest {
            workspace_root: workspace,
            paths: paths(params.paths),
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn list_lines(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsWorkspaceParams,
) -> Result<Vec<VcsLineOfWork>, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::LineList,
    )
    .await?;
    provider.list_lines(workspace).await.map_err(vcs_error)
}

pub(crate) async fn switch_line(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsLineSwitchParams,
) -> Result<VcsOperationResult, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        VcsOperation::LineSwitch,
    )
    .await?;
    provider
        .switch_line(VcsLineSwitchRequest {
            workspace_root: workspace,
            line_id: params.line_id,
        })
        .await
        .map_err(vcs_error)
}

pub(crate) async fn sync(
    resolver: RegistryVcsProviderResolver,
    runtime_workspace: Option<String>,
    params: VcsSyncParams,
) -> Result<VcsOperationResult, JsonRpcError> {
    let (provider, workspace) = resolve_provider(
        resolver,
        params.workspace,
        runtime_workspace,
        params.provider_id,
        sync_operation(params.operation.clone()),
    )
    .await?;
    provider
        .sync(VcsSyncRequest {
            workspace_root: workspace,
            operation: params.operation,
        })
        .await
        .map_err(vcs_error)
}

async fn resolve_provider(
    resolver: RegistryVcsProviderResolver,
    requested_workspace: Option<String>,
    runtime_workspace: Option<String>,
    preferred_provider_id: Option<String>,
    operation: VcsOperation,
) -> Result<(Arc<dyn VcsProvider>, PathBuf), JsonRpcError> {
    let workspace = workspace_path(requested_workspace, runtime_workspace)?;
    let resolution = resolver
        .resolve_provider(VcsResolveRequest {
            workspace_root: workspace.clone(),
            preferred_provider_id,
        })
        .await
        .map_err(vcs_error)?;
    match resolution {
        VcsProviderResolution::Available { provider, claim } => {
            Ok((provider, claim.workspace.root))
        }
        VcsProviderResolution::Unavailable { workspace_root } => {
            Err(vcs_error(VcsError::Unavailable {
                operation,
                provider_id: None,
                path: Some(workspace_root),
                message: "no version-control provider is available for this workspace".to_string(),
            }))
        }
    }
}

fn workspace_path(
    requested: Option<String>,
    runtime_workspace: Option<String>,
) -> Result<PathBuf, JsonRpcError> {
    let Some(runtime_workspace) = runtime_workspace else {
        return Err(invalid_params("workspace is required"));
    };
    let runtime = absolute_existing_workspace(&runtime_workspace, "runtime workspace")?;
    let Some(requested) = requested else {
        return Ok(runtime);
    };
    let requested = absolute_existing_workspace(&requested, "workspace")?;
    if requested != runtime && !requested.starts_with(&runtime) {
        return Err(invalid_params(
            "workspace must be the configured runtime workspace or a child path",
        ));
    }
    Ok(requested)
}

fn absolute_existing_workspace(path: &str, label: &str) -> Result<PathBuf, JsonRpcError> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(invalid_params("workspace must be absolute"));
    }
    path.canonicalize()
        .map_err(|err| invalid_params(format!("{label} is not accessible: {err}")))
}

fn paths(paths: Vec<String>) -> Vec<PathBuf> {
    paths.into_iter().map(PathBuf::from).collect()
}

fn totals(files: &[VcsChangedFile]) -> VcsChangesTotals {
    VcsChangesTotals {
        files: files.len() as u32,
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
    }
}

fn vcs_error(error: VcsError) -> JsonRpcError {
    let code = match error {
        VcsError::PathInvalid { .. } => -32602,
        VcsError::Unavailable { .. }
        | VcsError::UnsupportedOperation { .. }
        | VcsError::DirtyWorkspace { .. }
        | VcsError::CommandFailed { .. }
        | VcsError::ProviderNativeRequired { .. } => -32000,
    };
    JsonRpcError {
        code,
        message: error.to_string(),
        data: serde_json::to_value(&error).ok(),
    }
}

fn sync_operation(operation: VcsSyncOperation) -> VcsOperation {
    match operation {
        VcsSyncOperation::Fetch => VcsOperation::SyncFetch,
        VcsSyncOperation::Pull => VcsOperation::SyncPull,
        VcsSyncOperation::Push => VcsOperation::SyncPush,
    }
}

fn invalid_params(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: message.into(),
        data: None,
    }
}
