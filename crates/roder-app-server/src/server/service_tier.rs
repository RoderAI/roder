use super::*;

impl AppServer {
    pub(super) async fn handle_thread_set_service_tier(
        &self,
        params: ThreadSetServiceTierParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params
            .service_tier
            .as_ref()
            .is_some_and(|tier| tier.trim().is_empty())
        {
            return Err(invalid_params("serviceTier must be non-empty or null"));
        }
        if self
            .runtime
            .load_thread_metadata(&params.thread_id)
            .await
            .map_err(internal_error)?
            .is_none()
        {
            return Err(not_found(format!("thread not found: {}", params.thread_id)));
        }
        let service_tier = self
            .runtime
            .set_thread_service_tier(&params.thread_id, params.service_tier)
            .await
            .map_err(internal_error)?;
        if let Some(thread) = self
            .protocol_threads
            .write()
            .await
            .get_mut(&params.thread_id)
        {
            thread.service_tier = service_tier.clone();
        }
        Ok(serde_json::to_value(ThreadSetServiceTierResult {
            thread_id: params.thread_id,
            service_tier,
        })
        .unwrap())
    }
}
