use roder_api::thread::{ThreadHistoryReadOptions, ThreadHistoryTurn, ThreadHistoryTurnStatus};
use roder_protocol::{
    JsonRpcError, ThreadHistoryReadParams, ThreadHistoryReadResult, ThreadResumeParams,
    ThreadResumeResult, Turn,
};
use serde_json::Value;

use crate::server::{AppServer, internal_error};

impl AppServer {
    pub(crate) async fn handle_thread_history_read(
        &self,
        params: ThreadHistoryReadParams,
    ) -> Result<Value, JsonRpcError> {
        let result = self.read_thread_history_page(params).await?;
        serde_json::to_value(result).map_err(internal_error)
    }

    pub(crate) async fn handle_thread_resume(
        &self,
        params: ThreadResumeParams,
    ) -> Result<Value, JsonRpcError> {
        let request = ThreadHistoryReadParams {
            thread_id: params.thread_id,
            before: None,
            limit: params.limit,
            max_bytes: params.max_bytes,
        };
        let mut history = self.read_thread_history_page(request.clone()).await?;
        if history.thread.is_none() {
            // Older local threads can have an event log but no metadata file.
            // Loading metadata repairs that file before retrying the indexed read.
            if self
                .runtime
                .load_thread_metadata(&request.thread_id)
                .await
                .map_err(internal_error)?
                .is_some()
            {
                history = self.read_thread_history_page(request).await?;
            }
        }
        let lifecycle = if let Some(thread) = history.thread.take() {
            let lifecycle = self
                .runtime
                .resume_thread_history(&thread.id)
                .await
                .map_err(internal_error)?;
            history.thread = Some(self.thread_with_live_status(thread).await);
            lifecycle
        } else {
            Default::default()
        };
        serde_json::to_value(ThreadResumeResult { history, lifecycle }).map_err(internal_error)
    }

    async fn read_thread_history_page(
        &self,
        params: ThreadHistoryReadParams,
    ) -> Result<ThreadHistoryReadResult, JsonRpcError> {
        let default_limit = if params.before.is_some() { 120 } else { 160 };
        let page = self
            .runtime
            .read_thread_history(
                &params.thread_id,
                ThreadHistoryReadOptions {
                    before: params.before,
                    limit: params.limit.unwrap_or(default_limit).clamp(1, 200),
                    max_bytes: params
                        .max_bytes
                        .unwrap_or(2 * 1024 * 1024)
                        .clamp(1, 2 * 1024 * 1024),
                },
            )
            .await
            .map_err(internal_error)?;
        Ok(match page {
            Some(page) => {
                let turns = page.turns.into_iter().map(protocol_history_turn).collect();
                ThreadHistoryReadResult {
                    thread: Some(
                        self.protocol_thread_from_metadata_with_live_status(
                            page.metadata,
                            Some(turns),
                        )
                        .await,
                    ),
                    before: page.before,
                    has_more: page.has_more,
                }
            }
            None => ThreadHistoryReadResult {
                thread: None,
                before: None,
                has_more: false,
            },
        })
    }
}

fn protocol_history_turn(turn: ThreadHistoryTurn) -> Turn {
    Turn {
        id: turn.turn_id,
        items: turn.items.into_iter().map(Into::into).collect(),
        items_view: "default".to_string(),
        status: match turn.status {
            ThreadHistoryTurnStatus::InProgress => "inProgress",
            ThreadHistoryTurnStatus::Completed => "completed",
            ThreadHistoryTurnStatus::Failed => "failed",
            ThreadHistoryTurnStatus::Interrupted => "interrupted",
        }
        .to_string(),
        error: turn.error.map(Value::String),
        started_at: Some(turn.created_at.unix_timestamp()),
        completed_at: turn.completed_at.map(|time| time.unix_timestamp()),
        duration_ms: turn
            .completed_at
            .map(|time| (time - turn.created_at).whole_milliseconds().max(0) as i64),
        usage: turn.usage,
        finish_reason: turn.finish_reason,
    }
}
