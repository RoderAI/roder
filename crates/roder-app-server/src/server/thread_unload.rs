use super::*;

impl AppServer {
    pub(super) async fn handle_thread_unload(
        &self,
        params: ThreadUnloadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if !self
            .protocol_threads
            .read()
            .await
            .contains_key(&params.thread_id)
            && self
                .runtime
                .load_thread_metadata(&params.thread_id)
                .await
                .map_err(internal_error)?
                .is_none()
        {
            return Err(JsonRpcError {
                code: -32002,
                message: "thread not found".into(),
                data: None,
            });
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.runtime.unload_thread(&params.thread_id),
        )
        .await
        .map_err(internal_error)?
        .map_err(internal_error)?;
        self.protocol_threads
            .write()
            .await
            .remove(&params.thread_id);
        self.protocol_thread_models
            .write()
            .await
            .remove(&params.thread_id);
        roder_api::mcp_auth::clear_thread_token(&params.thread_id);
        Ok(serde_json::json!({}))
    }
}
