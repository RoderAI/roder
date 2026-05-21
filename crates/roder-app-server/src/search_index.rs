use std::path::{Path, PathBuf};

use roder_protocol::{
    JsonRpcError, JsonRpcNotification, SearchIndexClearParams, SearchIndexClearResult,
    SearchIndexRebuildParams, SearchIndexRebuildResult, SearchIndexStatus,
    SearchIndexStatusNotification, SearchIndexStatusParams, SearchIndexStatusResult,
    SearchIndexStatusState, SearchIndexWarmupParams, SearchIndexWarmupResult,
};
use roder_search::{
    SearchOptions, default_store_dir, load_persistent_index, manifest_path,
    rebuild_persistent_index,
};

use crate::server::AppServer;

impl AppServer {
    pub async fn handle_search_index_status(
        &self,
        params: SearchIndexStatusParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(SearchIndexStatusResult {
            status: self.search_index_status(params.workspace.as_deref()),
        })
        .unwrap())
    }

    pub async fn handle_search_index_warmup(
        &self,
        params: SearchIndexWarmupParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.search_index_workspace(params.workspace.as_deref());
        let store_dir = self.search_index_store_dir(&workspace);
        if !roder_search::search_index_enabled() {
            let status = self.search_index_status_for(
                SearchIndexStatusState::Disabled,
                &workspace,
                &store_dir,
                None,
            );
            self.publish_search_index_status(status.clone());
            return Ok(serde_json::to_value(SearchIndexWarmupResult { status }).unwrap());
        }

        if manifest_path(&store_dir).exists() {
            let status = self.search_index_status_for_workspace(&workspace, &store_dir);
            self.publish_search_index_status(status.clone());
            return Ok(serde_json::to_value(SearchIndexWarmupResult { status }).unwrap());
        }

        let status = self.rebuild_search_index(&workspace, &store_dir);
        Ok(serde_json::to_value(SearchIndexWarmupResult { status }).unwrap())
    }

    pub async fn handle_search_index_rebuild(
        &self,
        params: SearchIndexRebuildParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.search_index_workspace(params.workspace.as_deref());
        let store_dir = self.search_index_store_dir(&workspace);
        let status = self.rebuild_search_index(&workspace, &store_dir);
        Ok(serde_json::to_value(SearchIndexRebuildResult { status }).unwrap())
    }

    pub async fn handle_search_index_clear(
        &self,
        params: SearchIndexClearParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let workspace = self.search_index_workspace(params.workspace.as_deref());
        let store_dir = self.search_index_store_dir(&workspace);
        let status = match std::fs::remove_dir_all(&store_dir) {
            Ok(()) => self.search_index_status_for(
                SearchIndexStatusState::Cleared,
                &workspace,
                &store_dir,
                Some("persistent search index cleared".to_string()),
            ),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => self.search_index_status_for(
                SearchIndexStatusState::Cleared,
                &workspace,
                &store_dir,
                Some("persistent search index was already absent".to_string()),
            ),
            Err(err) => self.search_index_status_for(
                SearchIndexStatusState::Failed,
                &workspace,
                &store_dir,
                Some(format!("failed to clear persistent search index: {err}")),
            ),
        };
        self.publish_search_index_status(status.clone());
        Ok(serde_json::to_value(SearchIndexClearResult { status }).unwrap())
    }

    pub(crate) fn search_index_status(&self, workspace: Option<&str>) -> SearchIndexStatus {
        let workspace = self.search_index_workspace(workspace);
        let store_dir = self.search_index_store_dir(&workspace);
        self.search_index_status_for_workspace(&workspace, &store_dir)
    }

    pub(crate) fn publish_search_index_status(&self, status: SearchIndexStatus) {
        self.publish_notification(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "search_index/statusChanged".to_string(),
            params: serde_json::to_value(SearchIndexStatusNotification { status }).unwrap(),
        });
    }

    fn rebuild_search_index(&self, workspace: &Path, store_dir: &Path) -> SearchIndexStatus {
        let building = self.search_index_status_for(
            SearchIndexStatusState::Building,
            workspace,
            store_dir,
            Some("building persistent search index".to_string()),
        );
        self.publish_search_index_status(building);

        let options = SearchOptions::new("__roder_index_warmup__");
        let status = match rebuild_persistent_index(store_dir, workspace, &options) {
            Ok(stats) => SearchIndexStatus {
                state: SearchIndexStatusState::Ready,
                enabled: roder_search::search_index_enabled(),
                workspace: workspace.display().to_string(),
                store_dir: store_dir.display().to_string(),
                index_version: Some(stats.metadata.index_version),
                document_count: Some(stats.metadata.document_count as u64),
                index_bytes: Some(stats.metadata.index_bytes),
                build_time_ms: Some(stats.metadata.build_time_ms as u64),
                stale: false,
                message: Some(format!(
                    "indexed {} documents; changed {}; reused {}",
                    stats.metadata.document_count, stats.changed_documents, stats.reused_documents
                )),
            },
            Err(err) => self.search_index_status_for(
                SearchIndexStatusState::Failed,
                workspace,
                store_dir,
                Some(format!("failed to build persistent search index: {err}")),
            ),
        };
        self.publish_search_index_status(status.clone());
        status
    }

    fn search_index_status_for_workspace(
        &self,
        workspace: &Path,
        store_dir: &Path,
    ) -> SearchIndexStatus {
        if !roder_search::search_index_enabled() {
            return self.search_index_status_for(
                SearchIndexStatusState::Disabled,
                workspace,
                store_dir,
                Some("indexed search is disabled".to_string()),
            );
        }
        if !manifest_path(store_dir).exists() {
            return self.search_index_status_for(
                SearchIndexStatusState::Missing,
                workspace,
                store_dir,
                Some("persistent search index has not been built".to_string()),
            );
        }

        match load_persistent_index(store_dir, workspace, &SearchOptions::new("__status__")) {
            Ok(Some(loaded)) => {
                let stale = loaded.index.has_stale_documents();
                SearchIndexStatus {
                    state: if stale {
                        SearchIndexStatusState::Stale
                    } else {
                        SearchIndexStatusState::Ready
                    },
                    enabled: true,
                    workspace: workspace.display().to_string(),
                    store_dir: store_dir.display().to_string(),
                    index_version: Some(loaded.metadata.index_version),
                    document_count: Some(loaded.metadata.document_count as u64),
                    index_bytes: Some(loaded.metadata.index_bytes),
                    build_time_ms: Some(loaded.metadata.build_time_ms as u64),
                    stale,
                    message: if stale {
                        Some("persistent search index is stale; rebuild to refresh".to_string())
                    } else {
                        None
                    },
                }
            }
            Ok(None) => self.search_index_status_for(
                SearchIndexStatusState::Stale,
                workspace,
                store_dir,
                Some("persistent search index metadata no longer matches workspace".to_string()),
            ),
            Err(err) => self.search_index_status_for(
                SearchIndexStatusState::Failed,
                workspace,
                store_dir,
                Some(format!("failed to read persistent search index: {err}")),
            ),
        }
    }

    fn search_index_status_for(
        &self,
        state: SearchIndexStatusState,
        workspace: &Path,
        store_dir: &Path,
        message: Option<String>,
    ) -> SearchIndexStatus {
        let stale = matches!(state, SearchIndexStatusState::Stale);
        SearchIndexStatus {
            state,
            enabled: roder_search::search_index_enabled(),
            workspace: workspace.display().to_string(),
            store_dir: store_dir.display().to_string(),
            index_version: None,
            document_count: None,
            index_bytes: None,
            build_time_ms: None,
            stale,
            message,
        }
    }

    fn search_index_workspace(&self, workspace: Option<&str>) -> PathBuf {
        workspace
            .map(PathBuf::from)
            .unwrap_or_else(|| self.runtime.workspace())
    }

    fn search_index_store_dir(&self, workspace: &Path) -> PathBuf {
        let home = std::env::var_os("RODER_SEARCH_INDEX_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        default_store_dir(home, workspace)
    }
}
