use roder_api::artifacts::ContextArtifactAccess;
use roder_api::events::{ContextArtifactDeleted, RoderEvent};
use roder_protocol::{
    ArtifactDeleteParams, ArtifactDeleteResult, ArtifactGrepParams, ArtifactGrepResult,
    ArtifactListParams, ArtifactListResult, ArtifactReadParams, ArtifactReadResult,
    ArtifactTailParams, ArtifactTailResult, JsonRpcError,
};

use crate::server::AppServer;

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 200;

impl AppServer {
    pub(crate) async fn handle_artifact_list(
        &self,
        params: ArtifactListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut artifacts = self
            .runtime
            .context_artifacts()
            .list_artifacts(&params.thread_id)
            .map_err(internal_error)?;
        if let Some(kind) = params.kind {
            artifacts.retain(|artifact| artifact.kind == kind);
        }
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        artifacts.truncate(limit);
        Ok(serde_json::to_value(ArtifactListResult {
            artifacts: artifacts
                .into_iter()
                .map(|artifact| artifact.descriptor())
                .collect(),
        })
        .unwrap())
    }

    pub(crate) async fn handle_artifact_read(
        &self,
        params: ArtifactReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let page = self
            .runtime
            .context_artifacts()
            .read_artifact(
                &params.thread_id,
                &params.artifact_id,
                params.start_line.unwrap_or(1),
                clamp_limit(params.limit),
            )
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ArtifactReadResult { page }).unwrap())
    }

    pub(crate) async fn handle_artifact_grep(
        &self,
        params: ArtifactGrepParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let page = self
            .runtime
            .context_artifacts()
            .grep_artifact(
                &params.thread_id,
                &params.artifact_id,
                &params.query,
                params.offset.unwrap_or_default(),
                clamp_limit(params.limit),
            )
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ArtifactGrepResult { page }).unwrap())
    }

    pub(crate) async fn handle_artifact_tail(
        &self,
        params: ArtifactTailParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let page = self
            .runtime
            .context_artifacts()
            .tail_artifact(
                &params.thread_id,
                &params.artifact_id,
                clamp_limit(params.lines),
            )
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ArtifactTailResult { page }).unwrap())
    }

    pub(crate) async fn handle_artifact_delete(
        &self,
        params: ArtifactDeleteParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let deleted = self
            .runtime
            .context_artifacts()
            .delete_artifact(&params.thread_id, &params.artifact_id)
            .map_err(internal_error)?;
        if deleted {
            self.runtime
                .emit(RoderEvent::ContextArtifactDeleted(ContextArtifactDeleted {
                    thread_id: params.thread_id,
                    artifact_id: params.artifact_id,
                    timestamp: time::OffsetDateTime::now_utc(),
                }))
                .await;
        }
        Ok(serde_json::to_value(ArtifactDeleteResult { deleted }).unwrap())
    }
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}
