use super::*;

impl AppServer {
    pub(super) async fn handle_thread_state(
        &self,
        params: ThreadStateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mode = if let Some(id) = &params.thread_id {
            self.require_policy_thread(id).await?;
            self.runtime
                .thread_policy_mode(id)
                .await
                .map_err(internal_error)?;
            self.runtime.effective_policy_mode_for_thread(id).await
        } else {
            self.runtime.status().await.policy_mode
        };
        let pending = self
            .runtime
            .pending_plan_exit()
            .await
            .filter(|pending| {
                params
                    .thread_id
                    .as_ref()
                    .is_none_or(|id| id == &pending.thread_id)
            })
            .map(|pending| PendingPlanExitDescriptor {
                thread_id: pending.thread_id,
                turn_id: pending.turn_id,
                request_id: pending.request_id,
                target_mode: pending.target_mode,
                plan_summary: pending.plan_summary,
                next_steps: pending.next_steps,
                requested_at: pending.requested_at,
                expires_at: pending.expires_at,
            });
        Ok(serde_json::to_value(ThreadStateResult {
            mode,
            pending_plan_exit: pending,
        })
        .unwrap())
    }

    pub(super) async fn handle_thread_set_mode(
        &self,
        params: ThreadSetModeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if let Some(thread_id) = params.thread_id {
            self.require_policy_thread(&thread_id).await?;
            self.runtime
                .set_thread_policy_mode(&thread_id, params.mode)
                .await
                .map_err(internal_error)?;
            return serde_json::to_value(ThreadSetModeResult {
                mode: self
                    .runtime
                    .effective_policy_mode_for_thread(&thread_id)
                    .await,
            })
            .map_err(internal_error);
        }
        let cfg = self
            .runtime
            .set_policy_mode(params.mode, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadSetModeResult {
            mode: cfg.policy_mode,
        })
        .unwrap())
    }

    async fn require_policy_thread(&self, id: &str) -> Result<(), JsonRpcError> {
        if self.protocol_threads.read().await.contains_key(id)
            || self
                .runtime
                .load_thread_metadata(id)
                .await
                .map_err(internal_error)?
                .is_some()
        {
            return Ok(());
        }
        Err(not_found(format!("thread not found: {id}")))
    }
}
