use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ignore::WalkBuilder;
use roder_api::version_control::{
    RegistryVcsProviderResolver, VcsListFilesRequest, VcsProviderResolution, VcsProviderResolver,
    VcsResolveRequest,
};
use roder_protocol::{
    JsonRpcError, JsonRpcNotification, Workspace, WorkspaceFileEntry, WorkspaceFileKind,
    WorkspaceFileQueryMatch, WorkspaceFilesChildrenParams, WorkspaceFilesChildrenResult,
    WorkspaceFilesIndexState, WorkspaceFilesQueryParams, WorkspaceFilesQueryResult,
    WorkspaceFilesReadEncoding, WorkspaceFilesReadParams, WorkspaceFilesReadResult,
    WorkspaceFilesRebuildParams, WorkspaceFilesRebuildResult, WorkspaceFilesRootStatus,
    WorkspaceFilesStatus, WorkspaceFilesStatusNotification, WorkspaceFilesStatusParams,
    WorkspaceFilesStatusResult, WorkspaceRoot,
};
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::server::AppServer;

const DEFAULT_QUERY_LIMIT: usize = 50;
const MAX_QUERY_LIMIT: usize = 200;
const DEFAULT_READ_LIMIT: usize = 64 * 1024;
const MAX_READ_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) struct WorkspaceFileService {
    roots: RwLock<BTreeMap<String, RootCache>>,
    notifications: broadcast::Sender<JsonRpcNotification>,
    /// Serializes index builds so two concurrent requests never index the
    /// same root twice. Reads of an already-ready root skip the gate entirely.
    build_gate: Mutex<()>,
}

#[derive(Debug, Clone)]
struct RootCache {
    state: WorkspaceFilesIndexState,
    index: Option<Arc<RootIndex>>,
    build_time_ms: Option<u64>,
    indexed_at_ms: Option<i64>,
    message: Option<String>,
}

#[derive(Debug, Clone)]
struct RootIndex {
    files: Vec<IndexedFile>,
    directories: BTreeSet<String>,
    child_dirs: BTreeMap<String, BTreeSet<String>>,
    child_files: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedFile {
    path: String,
    name: String,
    size: u64,
    modified_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct QueryCandidate {
    entry: WorkspaceFileEntry,
    score: i64,
    match_positions: Vec<usize>,
}

impl WorkspaceFileService {
    pub(crate) fn new(notifications: broadcast::Sender<JsonRpcNotification>) -> Self {
        Self {
            roots: RwLock::new(BTreeMap::new()),
            notifications,
            build_gate: Mutex::new(()),
        }
    }

    async fn status(
        &self,
        workspace: &Workspace,
        root_id: Option<&str>,
    ) -> Result<WorkspaceFilesStatus, JsonRpcError> {
        let roots = selected_roots(workspace, root_id)?;
        Ok(self.status_for_roots(workspace, &roots).await)
    }

    fn publish(&self, status: &WorkspaceFilesStatus) {
        // A send error only means there are no subscribers; that is fine.
        let _ = self.notifications.send(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "workspace/files/statusChanged".to_string(),
            params: serde_json::to_value(WorkspaceFilesStatusNotification {
                status: status.clone(),
            })
            .unwrap(),
        });
    }

    async fn all_ready(&self, roots: &[WorkspaceRoot]) -> bool {
        let cache = self.roots.read().await;
        roots.iter().all(|root| {
            matches!(
                cache.get(&root.path).map(|entry| entry.state),
                Some(WorkspaceFilesIndexState::Ready | WorkspaceFilesIndexState::Stale)
            )
        })
    }

    /// Flag roots as building while preserving any previously built index, so
    /// reads keep serving the last-known tree until the rebuild lands.
    async fn mark_building(&self, roots: &[WorkspaceRoot]) {
        let mut cache = self.roots.write().await;
        for root in roots {
            let index = cache.get(&root.path).and_then(|entry| entry.index.clone());
            cache.insert(
                root.path.clone(),
                RootCache {
                    state: WorkspaceFilesIndexState::Building,
                    index,
                    build_time_ms: None,
                    indexed_at_ms: None,
                    message: Some("building workspace file index".to_string()),
                },
            );
        }
    }

    async fn build_roots(
        &self,
        resolver: &RegistryVcsProviderResolver,
        roots: &[WorkspaceRoot],
    ) -> Result<(), JsonRpcError> {
        for root in roots {
            let built = build_root_index(resolver, PathBuf::from(&root.path)).await;
            let cache_entry = match built {
                Ok(index) => RootCache {
                    state: WorkspaceFilesIndexState::Ready,
                    build_time_ms: Some(index.build_time_ms),
                    indexed_at_ms: Some(now_ms()),
                    message: Some(format!(
                        "indexed {} files and {} directories",
                        index.index.files.len(),
                        index.index.directories.len()
                    )),
                    index: Some(Arc::new(index.index)),
                },
                Err(err) => RootCache {
                    state: WorkspaceFilesIndexState::Failed,
                    index: None,
                    build_time_ms: None,
                    indexed_at_ms: Some(now_ms()),
                    message: Some(format!("failed to build workspace file index: {err:#}")),
                },
            };
            self.roots
                .write()
                .await
                .insert(root.path.clone(), cache_entry);
        }
        Ok(())
    }

    /// Build `roots` under the build gate. The gate serializes concurrent
    /// builds so the same root is never indexed twice; with `force` disabled an
    /// already-ready set returns immediately after the gate confirms it. Emits
    /// a `building` notification then a final `statusChanged` around the work.
    async fn build_under_gate(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        roots: &[WorkspaceRoot],
        force: bool,
    ) -> Result<WorkspaceFilesStatus, JsonRpcError> {
        let _gate = self.build_gate.lock().await;
        if !force && self.all_ready(roots).await {
            return Ok(self.status_for_roots(workspace, roots).await);
        }
        self.mark_building(roots).await;
        self.publish(&self.status_for_roots(workspace, roots).await);
        self.build_roots(resolver, roots).await?;
        let status = self.status_for_roots(workspace, roots).await;
        self.publish(&status);
        Ok(status)
    }

    async fn rebuild(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        root_id: Option<&str>,
    ) -> Result<WorkspaceFilesStatus, JsonRpcError> {
        let roots = selected_roots(workspace, root_id)?;
        self.build_under_gate(resolver, workspace, &roots, true)
            .await
    }

    async fn ensure_ready(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        root_id: Option<&str>,
    ) -> Result<WorkspaceFilesStatus, JsonRpcError> {
        let roots = selected_roots(workspace, root_id)?;
        if self.all_ready(&roots).await {
            return Ok(self.status_for_roots(workspace, &roots).await);
        }
        self.build_under_gate(resolver, workspace, &roots, false)
            .await
    }

    async fn children(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        root_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<WorkspaceFilesChildrenResult, JsonRpcError> {
        if root_id.is_none() {
            if path.as_ref().is_some_and(|path| !path.trim().is_empty()) {
                return Err(invalid_params(
                    "workspace/files/children path requires rootId",
                ));
            }
            let status = self.status(workspace, None).await?;
            let entries = workspace
                .roots
                .iter()
                .map(|root| WorkspaceFileEntry {
                    root_id: root.id.clone(),
                    root_name: root.name.clone(),
                    path: String::new(),
                    name: root.name.clone(),
                    kind: WorkspaceFileKind::Directory,
                    has_children: true,
                    size: None,
                    modified_ms: None,
                })
                .collect();
            return Ok(WorkspaceFilesChildrenResult { status, entries });
        }

        let relative_path = normalize_relative_path(path.unwrap_or_default(), true)?;
        let status = self.ensure_ready(resolver, workspace, root_id).await?;
        let root = selected_root(workspace, root_id)?;
        let index = self.ready_index(&root).await?;
        let mut entries = index.children(&root, &relative_path);
        let existing_dirs = entries
            .iter()
            .filter(|entry| entry.kind == WorkspaceFileKind::Directory)
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        // Empty directories have no indexed files, so they are absent from the
        // in-memory tree; a bounded depth-1 walk of just this directory is the
        // only way to surface them. It runs per expansion by necessity.
        let direct_dirs = {
            let root = root.clone();
            let relative_path = relative_path.clone();
            tokio::task::spawn_blocking(move || {
                direct_empty_dirs(&root, &relative_path, existing_dirs)
            })
            .await
            .map_err(|err| internal_error(format!("list workspace file children: {err}")))?
        };
        entries.extend(direct_dirs);
        sort_entries(&mut entries);
        Ok(WorkspaceFilesChildrenResult { status, entries })
    }

    async fn query(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        root_id: Option<&str>,
        query: &str,
        limit: Option<usize>,
    ) -> Result<WorkspaceFilesQueryResult, JsonRpcError> {
        let status = self.ensure_ready(resolver, workspace, root_id).await?;
        let roots = selected_roots(workspace, root_id)?;
        let limit = limit
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .clamp(1, MAX_QUERY_LIMIT);
        let mut candidates = Vec::new();
        for root in roots {
            let index = self.ready_index(&root).await?;
            candidates.extend(index.query(&root, query));
        }
        candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.entry.path.len().cmp(&right.entry.path.len()))
                .then_with(|| left.entry.path.cmp(&right.entry.path))
                .then_with(|| left.entry.root_id.cmp(&right.entry.root_id))
        });
        candidates.truncate(limit);
        let indexed_file_count = status.file_count;
        Ok(WorkspaceFilesQueryResult {
            status,
            matches: candidates
                .into_iter()
                .map(|candidate| WorkspaceFileQueryMatch {
                    entry: candidate.entry,
                    score: candidate.score,
                    match_positions: candidate.match_positions,
                })
                .collect(),
            indexed_file_count,
        })
    }

    async fn read(
        &self,
        resolver: &RegistryVcsProviderResolver,
        workspace: &Workspace,
        root_id: &str,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<WorkspaceFilesReadResult, JsonRpcError> {
        self.ensure_ready(resolver, workspace, Some(root_id))
            .await?;
        let root = selected_root(workspace, Some(root_id))?;
        let relative_path = normalize_relative_path(path, false)?;
        let index = self.ready_index(&root).await?;
        let indexed = index
            .file(&relative_path)
            .ok_or_else(|| invalid_params("workspace file is not indexed"))?;
        let entry = indexed.entry(&root);
        let read_limit = limit.unwrap_or(DEFAULT_READ_LIMIT).clamp(1, MAX_READ_LIMIT);
        let root = root.clone();
        let read_offset = offset.unwrap_or(0);
        tokio::task::spawn_blocking(move || {
            read_file_preview(&root, &relative_path, entry, read_offset, read_limit)
        })
        .await
        .map_err(|err| internal_error(format!("read workspace file preview: {err}")))?
    }

    async fn ready_index(&self, root: &WorkspaceRoot) -> Result<Arc<RootIndex>, JsonRpcError> {
        let cache = self.roots.read().await;
        cache
            .get(&root.path)
            .and_then(|entry| entry.index.clone())
            .ok_or_else(|| invalid_params("workspace file index is not ready"))
    }

    async fn status_for_roots(
        &self,
        workspace: &Workspace,
        roots: &[WorkspaceRoot],
    ) -> WorkspaceFilesStatus {
        let cache = self.roots.read().await;
        let root_statuses = roots
            .iter()
            .map(|root| root_status(root, cache.get(&root.path)))
            .collect::<Vec<_>>();
        aggregate_status(workspace, root_statuses)
    }
}

impl RootIndex {
    fn new(mut files: Vec<IndexedFile>) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        files.dedup_by(|left, right| left.path == right.path);
        let mut directories = BTreeSet::new();
        let mut child_dirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut child_files: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for (index, file) in files.iter().enumerate() {
            let parent = parent_path(&file.path);
            child_files.entry(parent.clone()).or_default().push(index);
            for dir in ancestor_dirs(&file.path) {
                let parent = parent_path(&dir);
                directories.insert(dir.clone());
                child_dirs.entry(parent).or_default().insert(dir);
            }
        }

        Self {
            files,
            directories,
            child_dirs,
            child_files,
        }
    }

    fn children(&self, root: &WorkspaceRoot, path: &str) -> Vec<WorkspaceFileEntry> {
        let mut entries = Vec::new();
        if let Some(dirs) = self.child_dirs.get(path) {
            entries.extend(dirs.iter().map(|dir| WorkspaceFileEntry {
                root_id: root.id.clone(),
                root_name: root.name.clone(),
                path: dir.clone(),
                name: basename(dir).to_string(),
                kind: WorkspaceFileKind::Directory,
                has_children: self.has_children(dir),
                size: None,
                modified_ms: None,
            }));
        }
        if let Some(files) = self.child_files.get(path) {
            entries.extend(
                files
                    .iter()
                    .map(|file_index| self.files[*file_index].entry(root)),
            );
        }
        entries
    }

    fn query(&self, root: &WorkspaceRoot, query: &str) -> Vec<QueryCandidate> {
        let mut candidates = self
            .directories
            .iter()
            .filter_map(|directory| {
                let name = basename(directory);
                let (score, match_positions) = rank_path(query, directory, name)?;
                Some(QueryCandidate {
                    entry: self.directory_entry(root, directory),
                    score,
                    match_positions,
                })
            })
            .collect::<Vec<_>>();
        candidates.extend(self.files.iter().filter_map(|file| {
            let (score, match_positions) = rank_path(query, &file.path, &file.name)?;
            Some(QueryCandidate {
                entry: file.entry(root),
                score,
                match_positions,
            })
        }));
        candidates
    }

    fn directory_entry(&self, root: &WorkspaceRoot, path: &str) -> WorkspaceFileEntry {
        WorkspaceFileEntry {
            root_id: root.id.clone(),
            root_name: root.name.clone(),
            path: path.to_string(),
            name: basename(path).to_string(),
            kind: WorkspaceFileKind::Directory,
            has_children: self.has_children(path),
            size: None,
            modified_ms: None,
        }
    }

    fn file(&self, path: &str) -> Option<&IndexedFile> {
        self.files
            .binary_search_by(|file| file.path.as_str().cmp(path))
            .ok()
            .map(|index| &self.files[index])
    }

    fn has_children(&self, dir: &str) -> bool {
        self.child_dirs
            .get(dir)
            .is_some_and(|dirs| !dirs.is_empty())
            || self
                .child_files
                .get(dir)
                .is_some_and(|files| !files.is_empty())
    }
}

impl IndexedFile {
    fn entry(&self, root: &WorkspaceRoot) -> WorkspaceFileEntry {
        WorkspaceFileEntry {
            root_id: root.id.clone(),
            root_name: root.name.clone(),
            path: self.path.clone(),
            name: self.name.clone(),
            kind: WorkspaceFileKind::File,
            has_children: false,
            size: Some(self.size),
            modified_ms: self.modified_ms,
        }
    }
}

struct BuiltRootIndex {
    index: RootIndex,
    build_time_ms: u64,
}

async fn build_root_index(
    resolver: &RegistryVcsProviderResolver,
    root: PathBuf,
) -> anyhow::Result<BuiltRootIndex> {
    let start = Instant::now();
    let canonical = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || root.canonicalize()).await??
    };
    // Prefer the workspace's resolved VCS provider so enumeration honors that
    // VCS's own tracking and ignore rules. Providers that cannot enumerate
    // (or workspaces with no provider) fall back to an ignore-aware walk.
    let provider_files = enumerate_via_provider(resolver, &canonical).await;
    let files = {
        let canonical = canonical.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<IndexedFile>> {
            match provider_files {
                Some(absolute_paths) => {
                    let mut files = Vec::new();
                    for absolute in absolute_paths {
                        if let Some(file) = indexed_file(&canonical, &absolute)? {
                            files.push(file);
                        }
                    }
                    Ok(files)
                }
                None => enumerate_walk_files(&canonical),
            }
        })
        .await??
    };
    Ok(BuiltRootIndex {
        index: RootIndex::new(files),
        build_time_ms: start.elapsed().as_millis() as u64,
    })
}

/// Ask the workspace's resolved VCS provider to enumerate its files. Returns
/// `None` when there is no available provider or the provider does not support
/// file listing, signalling the caller to fall back to a filesystem walk.
async fn enumerate_via_provider(
    resolver: &RegistryVcsProviderResolver,
    root: &Path,
) -> Option<Vec<PathBuf>> {
    let resolution = resolver
        .resolve_provider(VcsResolveRequest {
            workspace_root: root.to_path_buf(),
            preferred_provider_id: None,
        })
        .await
        .ok()?;
    let VcsProviderResolution::Available { provider, .. } = resolution else {
        return None;
    };
    match provider
        .list_files(VcsListFilesRequest {
            workspace_root: root.to_path_buf(),
        })
        .await
    {
        Ok(listing) => Some(listing.files),
        Err(_) => None,
    }
}

fn enumerate_walk_files(root: &Path) -> anyhow::Result<Vec<IndexedFile>> {
    let mut files = Vec::new();
    let mut walk = WalkBuilder::new(root);
    walk.standard_filters(true)
        .hidden(false)
        .require_git(false)
        .filter_entry({
            let root = root.to_path_buf();
            move |entry| !ignored_path(&root, entry.path())
        })
        .sort_by_file_path(|left, right| left.cmp(right));

    for entry in walk.build() {
        let entry = entry.map_err(std::io::Error::other)?;
        if let Some(file) = indexed_file(root, entry.path())? {
            files.push(file);
        }
    }
    Ok(files)
}

fn indexed_file(root: &Path, path: &Path) -> anyhow::Result<Option<IndexedFile>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let Some(relative_path) = relative_path(root, path)? else {
        return Ok(None);
    };
    if relative_path.is_empty() || ignored_relative_path(&relative_path) {
        return Ok(None);
    }
    Ok(Some(IndexedFile {
        name: basename(&relative_path).to_string(),
        path: relative_path,
        size: metadata.len(),
        modified_ms: modified_ms(&metadata),
    }))
}

fn relative_path(root: &Path, path: &Path) -> anyhow::Result<Option<String>> {
    let absolute = match path.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if !absolute.starts_with(&root) {
        return Ok(None);
    }
    Ok(Some(path_to_slash(
        absolute
            .strip_prefix(root)?
            .components()
            .collect::<PathBuf>(),
    )))
}

fn selected_root(
    workspace: &Workspace,
    root_id: Option<&str>,
) -> Result<WorkspaceRoot, JsonRpcError> {
    let selected = root_id.unwrap_or(&workspace.default_root_id);
    workspace
        .roots
        .iter()
        .find(|root| root.id == selected)
        .cloned()
        .ok_or_else(|| invalid_params("unknown rootId"))
}

fn selected_roots(
    workspace: &Workspace,
    root_id: Option<&str>,
) -> Result<Vec<WorkspaceRoot>, JsonRpcError> {
    match root_id {
        Some(root_id) => Ok(vec![selected_root(workspace, Some(root_id))?]),
        None => Ok(workspace.roots.clone()),
    }
}

fn root_status(root: &WorkspaceRoot, cache: Option<&RootCache>) -> WorkspaceFilesRootStatus {
    let (state, stale, file_count, directory_count, build_time_ms, indexed_at_ms, message) =
        match cache {
            Some(cache) => (
                cache.state,
                matches!(cache.state, WorkspaceFilesIndexState::Stale),
                cache.index.as_ref().map(|index| index.files.len() as u64),
                cache
                    .index
                    .as_ref()
                    .map(|index| index.directories.len() as u64),
                cache.build_time_ms,
                cache.indexed_at_ms,
                cache.message.clone(),
            ),
            None => (
                WorkspaceFilesIndexState::Missing,
                false,
                None,
                None,
                None,
                None,
                Some("workspace file index has not been built".to_string()),
            ),
        };
    WorkspaceFilesRootStatus {
        root_id: root.id.clone(),
        root_name: root.name.clone(),
        state,
        stale,
        file_count,
        directory_count,
        build_time_ms,
        indexed_at_ms,
        message,
    }
}

fn aggregate_status(
    workspace: &Workspace,
    roots: Vec<WorkspaceFilesRootStatus>,
) -> WorkspaceFilesStatus {
    let state = if roots
        .iter()
        .any(|root| root.state == WorkspaceFilesIndexState::Building)
    {
        WorkspaceFilesIndexState::Building
    } else if roots
        .iter()
        .any(|root| root.state == WorkspaceFilesIndexState::Failed)
    {
        WorkspaceFilesIndexState::Failed
    } else if roots
        .iter()
        .any(|root| root.state == WorkspaceFilesIndexState::Missing)
    {
        WorkspaceFilesIndexState::Missing
    } else if roots
        .iter()
        .any(|root| root.state == WorkspaceFilesIndexState::Stale)
    {
        WorkspaceFilesIndexState::Stale
    } else {
        WorkspaceFilesIndexState::Ready
    };
    let file_count = roots.iter().filter_map(|root| root.file_count).sum();
    let directory_count = roots.iter().filter_map(|root| root.directory_count).sum();
    let stale = roots.iter().any(|root| root.stale);
    let message = if state == WorkspaceFilesIndexState::Ready {
        None
    } else {
        roots.iter().find_map(|root| root.message.clone())
    };
    WorkspaceFilesStatus {
        workspace_id: workspace.id.clone(),
        state,
        stale,
        roots,
        file_count,
        directory_count,
        message,
    }
}

/// Surface directories that have no indexed files beneath them (empty or
/// ignored-only) so the tree can still show empty folders. Uses the same
/// ignore-aware walk as enumeration so a directory the VCS ignores is not
/// revealed here even though it was correctly excluded from the file index.
fn direct_empty_dirs(
    root: &WorkspaceRoot,
    relative_path: &str,
    existing_dirs: BTreeSet<String>,
) -> Vec<WorkspaceFileEntry> {
    let Ok(path) = scoped_existing_dir(root, relative_path) else {
        return Vec::new();
    };
    let mut walk = WalkBuilder::new(&path);
    walk.standard_filters(true)
        .hidden(false)
        .require_git(false)
        .max_depth(Some(1));
    let mut entries = Vec::new();
    for entry in walk.build().flatten() {
        // Depth 0 is the directory being listed; we only want its children.
        if entry.depth() == 0 {
            continue;
        }
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let child_path = join_relative(relative_path, &name);
        if existing_dirs.contains(&child_path) || ignored_relative_path(&child_path) {
            continue;
        }
        entries.push(WorkspaceFileEntry {
            root_id: root.id.clone(),
            root_name: root.name.clone(),
            path: child_path,
            name,
            kind: WorkspaceFileKind::Directory,
            has_children: true,
            size: None,
            modified_ms: entry.metadata().ok().and_then(|m| modified_ms(&m)),
        });
    }
    entries
}

enum TextWindow {
    Text { skipped: usize, text: String },
    Binary,
    Unsupported,
}

fn read_file_preview(
    root: &WorkspaceRoot,
    relative_path: &str,
    entry: WorkspaceFileEntry,
    offset: usize,
    read_limit: usize,
) -> Result<WorkspaceFilesReadResult, JsonRpcError> {
    let path = scoped_existing_file(root, relative_path)?;
    let metadata = std::fs::metadata(&path).map_err(internal_error)?;
    let total_bytes = metadata.len();
    if obvious_binary(&path) {
        return Ok(binary_read_result(entry, offset, read_limit, total_bytes));
    }
    if offset as u64 >= total_bytes {
        // At or past EOF: an empty, final page.
        return Ok(WorkspaceFilesReadResult {
            entry,
            encoding: WorkspaceFilesReadEncoding::Utf8,
            text: Some(String::new()),
            offset,
            limit: read_limit,
            total_bytes,
            has_more: false,
            truncated: false,
        });
    }

    // Read only this page's window straight from `offset` so pagination works
    // for files of any size and never re-reads the whole file.
    let mut file = std::fs::File::open(&path).map_err(internal_error)?;
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(internal_error)?;
    let mut window = vec![0_u8; read_limit];
    let read = file.read(&mut window).map_err(internal_error)?;
    window.truncate(read);

    match decode_text_window(&window) {
        TextWindow::Binary => Ok(binary_read_result(entry, offset, read_limit, total_bytes)),
        TextWindow::Unsupported => Ok(WorkspaceFilesReadResult {
            entry,
            encoding: WorkspaceFilesReadEncoding::Unsupported,
            text: None,
            offset,
            limit: read_limit,
            total_bytes,
            has_more: false,
            truncated: false,
        }),
        TextWindow::Text { skipped, text } => {
            let start = offset + skipped;
            let end = (start + text.len()) as u64;
            Ok(WorkspaceFilesReadResult {
                entry,
                encoding: WorkspaceFilesReadEncoding::Utf8,
                offset: start,
                limit: read_limit,
                total_bytes,
                has_more: end < total_bytes,
                truncated: false,
                text: Some(text),
            })
        }
    }
}

fn binary_read_result(
    entry: WorkspaceFileEntry,
    offset: usize,
    read_limit: usize,
    total_bytes: u64,
) -> WorkspaceFilesReadResult {
    WorkspaceFilesReadResult {
        entry,
        encoding: WorkspaceFilesReadEncoding::Binary,
        text: None,
        offset,
        limit: read_limit,
        total_bytes,
        has_more: false,
        truncated: false,
    }
}

/// Decode a byte window read from an arbitrary file offset into UTF-8 text.
/// Because a window can start or end mid-character, leading continuation bytes
/// are skipped and an incomplete trailing character is dropped; a real invalid
/// sequence or a NUL byte marks the file as binary/unsupported instead.
fn decode_text_window(bytes: &[u8]) -> TextWindow {
    if bytes.contains(&0) {
        return TextWindow::Binary;
    }
    let mut skipped = 0;
    while skipped < bytes.len() && (bytes[skipped] & 0xC0) == 0x80 {
        skipped += 1;
    }
    let slice = &bytes[skipped..];
    match std::str::from_utf8(slice) {
        Ok(text) => TextWindow::Text {
            skipped,
            text: text.to_string(),
        },
        // An incomplete final character at the window edge is expected during
        // pagination: keep the valid prefix.
        Err(err) if err.error_len().is_none() => TextWindow::Text {
            skipped,
            text: std::str::from_utf8(&slice[..err.valid_up_to()])
                .unwrap()
                .to_string(),
        },
        // A genuine invalid byte sequence: not UTF-8 text.
        Err(_) => TextWindow::Unsupported,
    }
}

fn scoped_existing_dir(root: &WorkspaceRoot, relative_path: &str) -> Result<PathBuf, JsonRpcError> {
    let root_path = PathBuf::from(&root.path);
    let joined = root_path.join(relative_path);
    let canonical = joined.canonicalize().map_err(internal_error)?;
    if !canonical.starts_with(&root_path) || !canonical.is_dir() {
        return Err(invalid_params("path must be a directory inside root"));
    }
    Ok(canonical)
}

fn scoped_existing_file(
    root: &WorkspaceRoot,
    relative_path: &str,
) -> Result<PathBuf, JsonRpcError> {
    let root_path = PathBuf::from(&root.path);
    let joined = root_path.join(relative_path);
    let canonical = joined.canonicalize().map_err(internal_error)?;
    if !canonical.starts_with(&root_path) || !canonical.is_file() {
        return Err(invalid_params("path must be a file inside root"));
    }
    Ok(canonical)
}

fn normalize_relative_path(path: &str, allow_empty: bool) -> Result<String, JsonRpcError> {
    if path.trim().is_empty() {
        return if allow_empty {
            Ok(String::new())
        } else {
            Err(invalid_params("path must not be empty"))
        };
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(invalid_params("path must be relative to workspace root"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_params("path must stay inside workspace root"));
            }
        }
    }
    let normalized = path_to_slash(normalized);
    if normalized.is_empty() && !allow_empty {
        return Err(invalid_params("path must not be empty"));
    }
    Ok(normalized)
}

fn path_to_slash(path: PathBuf) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn sort_entries(entries: &mut [WorkspaceFileEntry]) {
    entries.sort_by(|left, right| {
        entry_kind_order(left)
            .cmp(&entry_kind_order(right))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.path.cmp(&right.path))
    });
}

fn entry_kind_order(entry: &WorkspaceFileEntry) -> u8 {
    match entry.kind {
        WorkspaceFileKind::Directory => 0,
        WorkspaceFileKind::File => 1,
    }
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn ancestor_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut current = String::new();
    let parts = path.split('/').collect::<Vec<_>>();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        current = join_relative(&current, part);
        dirs.push(current.clone());
    }
    dirs
}

fn join_relative(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn rank_path(query: &str, path: &str, name: &str) -> Option<(i64, Vec<usize>)> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let path = path.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let segments = path.split('/').collect::<Vec<_>>();
    let mut score = 0;
    let mut positions = BTreeSet::new();
    for term in query.split_whitespace() {
        let term_positions = subsequence_positions(&path, term)?;
        positions.extend(term_positions);
        if name == term {
            score += 1000;
        } else if segments.iter().any(|segment| *segment == term) {
            score += 800;
        } else if name.contains(term) {
            score += 650;
        } else if path.contains(term) {
            score += 400;
        } else {
            score += 100;
        }
    }
    if name == query {
        score += 500;
    }
    if path.contains(&query) {
        score += 250;
    }
    score -= (path.len() as i64).min(200);
    Some((score, positions.into_iter().collect()))
}

fn subsequence_positions(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    let mut positions = Vec::new();
    let mut chars = needle.chars();
    let mut current = chars.next()?;
    for (index, candidate) in haystack.char_indices() {
        if candidate == current {
            positions.push(index);
            match chars.next() {
                Some(next) => current = next,
                None => return Some(positions),
            }
        }
    }
    None
}

fn ignored_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    ignored_relative_path(&path_to_slash(relative.components().collect::<PathBuf>()))
}

fn ignored_relative_path(path: &str) -> bool {
    path.split('/').any(|component| {
        matches!(
            component,
            ".cache"
                | ".git"
                | ".gradle"
                | ".next"
                | ".nuxt"
                | ".parcel-cache"
                | ".roder"
                | ".svelte-kit"
                | ".turbo"
                | ".vite"
                | ".yarn"
                | "DerivedData"
                | "Pods"
                | "build"
                | "coverage"
                | "dist"
                | "node_modules"
                | "out"
                | "target"
        )
    })
}

fn obvious_binary(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "7z" | "a"
                    | "bin"
                    | "bmp"
                    | "class"
                    | "db"
                    | "dylib"
                    | "exe"
                    | "gif"
                    | "gz"
                    | "ico"
                    | "jar"
                    | "jpeg"
                    | "jpg"
                    | "o"
                    | "pdf"
                    | "png"
                    | "sqlite"
                    | "tar"
                    | "ttf"
                    | "wasm"
                    | "webp"
                    | "woff"
                    | "woff2"
                    | "zip"
            )
        })
}

fn modified_ms(metadata: &std::fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
}

fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

impl AppServer {
    async fn resolve_workspace_files_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<Workspace, JsonRpcError> {
        let runtime_workspace = self.runtime.status().await.workspace;
        self.workspaces
            .resolve_workspace(runtime_workspace, workspace_id)
            .await
    }

    pub async fn handle_workspace_files_status(
        &self,
        params: WorkspaceFilesStatusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self
            .resolve_workspace_files_workspace(&params.workspace_id)
            .await?;
        Ok(serde_json::to_value(WorkspaceFilesStatusResult {
            status: self
                .workspace_files
                .status(&workspace, params.root_id.as_deref())
                .await?,
        })
        .unwrap())
    }

    pub async fn handle_workspace_files_rebuild(
        &self,
        params: WorkspaceFilesRebuildParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self
            .resolve_workspace_files_workspace(&params.workspace_id)
            .await?;
        let resolver = self.runtime.registry().version_control_resolver();
        let status = self
            .workspace_files
            .rebuild(&resolver, &workspace, params.root_id.as_deref())
            .await?;
        Ok(serde_json::to_value(WorkspaceFilesRebuildResult { status }).unwrap())
    }

    pub async fn handle_workspace_files_children(
        &self,
        params: WorkspaceFilesChildrenParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self
            .resolve_workspace_files_workspace(&params.workspace_id)
            .await?;
        let resolver = self.runtime.registry().version_control_resolver();
        Ok(serde_json::to_value(
            self.workspace_files
                .children(
                    &resolver,
                    &workspace,
                    params.root_id.as_deref(),
                    params.path.as_deref(),
                )
                .await?,
        )
        .unwrap())
    }

    pub async fn handle_workspace_files_query(
        &self,
        params: WorkspaceFilesQueryParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self
            .resolve_workspace_files_workspace(&params.workspace_id)
            .await?;
        let resolver = self.runtime.registry().version_control_resolver();
        Ok(serde_json::to_value(
            self.workspace_files
                .query(
                    &resolver,
                    &workspace,
                    params.root_id.as_deref(),
                    &params.query,
                    params.limit,
                )
                .await?,
        )
        .unwrap())
    }

    pub async fn handle_workspace_files_read(
        &self,
        params: WorkspaceFilesReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self
            .resolve_workspace_files_workspace(&params.workspace_id)
            .await?;
        let resolver = self.runtime.registry().version_control_resolver();
        Ok(serde_json::to_value(
            self.workspace_files
                .read(
                    &resolver,
                    &workspace,
                    &params.root_id,
                    &params.path,
                    params.offset,
                    params.limit,
                )
                .await?,
        )
        .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_index_synthesizes_directories_from_files() {
        let index = RootIndex::new(vec![
            IndexedFile {
                path: "roadmap/status.md".to_string(),
                name: "status.md".to_string(),
                size: 1,
                modified_ms: None,
            },
            IndexedFile {
                path: "src/main.rs".to_string(),
                name: "main.rs".to_string(),
                size: 1,
                modified_ms: None,
            },
        ]);
        assert!(index.directories.contains("roadmap"));
        assert!(index.directories.contains("src"));
        assert_eq!(
            index.child_dirs[""].iter().cloned().collect::<Vec<_>>(),
            vec!["roadmap".to_string(), "src".to_string()]
        );
    }

    #[test]
    fn normalize_relative_path_rejects_escape_paths() {
        assert!(normalize_relative_path("roadmap/status.md", false).is_ok());
        assert!(normalize_relative_path("/tmp/status.md", false).is_err());
        assert!(normalize_relative_path("../status.md", false).is_err());
        assert!(normalize_relative_path("", false).is_err());
        assert_eq!(normalize_relative_path("", true).unwrap(), "");
    }

    #[test]
    fn ranked_query_prefers_exact_basename() {
        let exact = IndexedFile {
            path: "docs/status.md".to_string(),
            name: "status.md".to_string(),
            size: 1,
            modified_ms: None,
        };
        let weak = IndexedFile {
            path: "docs/state-updates.md".to_string(),
            name: "state-updates.md".to_string(),
            size: 1,
            modified_ms: None,
        };
        let exact_score = rank_path("status.md", &exact.path, &exact.name).unwrap().0;
        let weak_score = rank_path("status.md", &weak.path, &weak.name)
            .map(|rank| rank.0)
            .unwrap_or(i64::MIN);
        assert!(exact_score > weak_score);
    }

    #[test]
    fn decode_text_window_handles_boundaries_and_binary() {
        // A clean window decodes whole.
        let TextWindow::Text { skipped, text } = decode_text_window("αβγ".as_bytes()) else {
            panic!("expected text");
        };
        assert_eq!(skipped, 0);
        assert_eq!(text, "αβγ");

        // "αβ" is four bytes (α = 0..2, β = 2..4); a window that starts inside α
        // skips the dangling continuation byte and resumes at β.
        let bytes = "αβ".as_bytes();
        let TextWindow::Text { skipped, text } = decode_text_window(&bytes[1..]) else {
            panic!("expected text");
        };
        assert_eq!(skipped, 1);
        assert_eq!(text, "β");

        // A window that ends mid-character drops the incomplete trailing char.
        let TextWindow::Text { skipped, text } = decode_text_window(&bytes[..3]) else {
            panic!("expected text");
        };
        assert_eq!(skipped, 0);
        assert_eq!(text, "α");

        assert!(matches!(
            decode_text_window(&[b'a', 0, b'b']),
            TextWindow::Binary
        ));
        assert!(matches!(
            decode_text_window(&[0xFF, 0xFE, b'a']),
            TextWindow::Unsupported
        ));
    }

    #[test]
    fn fallback_walk_indexes_non_git_files() {
        let root = temp_root("fallback-walk");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/readme.md"), "hello").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "ignored").unwrap();
        // Roots are canonicalized before enumeration (build_root_index does this);
        // do the same here so symlinked temp dirs (e.g. macOS /var -> /private/var)
        // do not get filtered out by the in-root path check.
        let root = root.canonicalize().unwrap();

        let files = enumerate_walk_files(&root).unwrap();
        let paths = files.into_iter().map(|file| file.path).collect::<Vec<_>>();
        assert_eq!(paths, ["docs/readme.md"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_empty_dirs_hides_ignored_directories() {
        let root_dir = temp_root("empty-dirs");
        std::fs::create_dir_all(root_dir.join("visible-dir")).unwrap();
        std::fs::create_dir_all(root_dir.join("ignored-dir")).unwrap();
        std::fs::write(root_dir.join(".gitignore"), "ignored-dir/\n").unwrap();
        let canonical = root_dir.canonicalize().unwrap();
        let root = WorkspaceRoot {
            id: "root-1".to_string(),
            path: canonical.to_string_lossy().to_string(),
            name: "repo".to_string(),
        };

        let names = direct_empty_dirs(&root, "", BTreeSet::new())
            .into_iter()
            .map(|entry| entry.name)
            .collect::<BTreeSet<_>>();

        assert!(names.contains("visible-dir"));
        assert!(!names.contains("ignored-dir"));
        let _ = std::fs::remove_dir_all(root_dir);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "roder-workspace-files-{prefix}-{}",
            uuid::Uuid::new_v4()
        ))
    }
}
