use roder_api::goals::ThreadGoalPatch;
use roder_protocol::{
    JsonRpcError, ThreadGoalClearParams, ThreadGoalClearResult, ThreadGoalGetParams,
    ThreadGoalGetResult, ThreadGoalSetParams, ThreadGoalSetResult,
};

use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_thread_goal_get(
        &self,
        params: ThreadGoalGetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let goal = self
            .runtime
            .thread_goal_get(&params.thread_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadGoalGetResult { goal }).unwrap())
    }

    pub(crate) async fn handle_thread_goal_set(
        &self,
        params: ThreadGoalSetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let goal = self
            .runtime
            .thread_goal_set(
                &params.thread_id,
                ThreadGoalPatch {
                    objective: params.objective,
                    status: params.status,
                    token_budget: params.token_budget,
                },
            )
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadGoalSetResult { goal }).unwrap())
    }

    pub(crate) async fn handle_thread_goal_clear(
        &self,
        params: ThreadGoalClearParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cleared = self
            .runtime
            .thread_goal_clear(&params.thread_id)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ThreadGoalClearResult { cleared }).unwrap())
    }
}
