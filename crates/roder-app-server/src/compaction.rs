use roder_protocol::{JsonRpcError, ThreadCompactParams, ThreadCompactResult};

use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_thread_compact(
        &self,
        params: ThreadCompactParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .force_compact_thread(&params.thread_id, &params.turn_id, params.preserve_hint)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadCompactResult {
            compacted: outcome.compacted,
            reason: outcome.reason,
            estimated_tokens_before: outcome.estimated_tokens_before,
            estimated_tokens_after: outcome.estimated_tokens_after,
        })
        .unwrap())
    }
}
