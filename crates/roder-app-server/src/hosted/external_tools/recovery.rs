//! Recover execution receipts without replaying browser actions after host loss.
use super::ExecutorBindings;
use roder_api::events::{ExternalToolCallOutcome, RoderEvent};
use roder_protocol::{ToolExecutionState, ToolExecutorLease};

impl ExecutorBindings {
    pub async fn read_with_history(
        &self,
        connection: &str,
        lease: &ToolExecutorLease,
        request: &str,
        runtime: &roder_core::Runtime,
    ) -> Result<Option<ToolExecutionState>, String> {
        if let Some(state) = self.read(connection, lease, request).await? {
            return Ok(Some(state));
        }
        let snapshot = runtime
            .load_thread(&lease.thread_id)
            .await
            .map_err(|_| "execution_history_unavailable")?;
        let mut restored = None;
        if let Some(snapshot) = snapshot {
            for envelope in snapshot.events {
                match envelope.event {
                    RoderEvent::ExternalToolCallRequested(event)
                        if event.thread_id == lease.thread_id && event.request_id == request =>
                    {
                        restored = Some(ToolExecutionState {
                            thread_id: event.thread_id,
                            turn_id: event.turn_id,
                            request_id: event.request_id,
                            tool_name: event.tool_name,
                            state: "uncertain".into(),
                            is_error: true,
                        });
                    }
                    RoderEvent::ExternalToolCallResolved(event)
                        if event.thread_id == lease.thread_id && event.request_id == request =>
                    {
                        restored = Some(ToolExecutionState {
                            thread_id: event.thread_id,
                            turn_id: event.turn_id,
                            request_id: event.request_id,
                            tool_name: event.tool_name,
                            state: match event.outcome {
                                ExternalToolCallOutcome::Resolved => "resolved",
                                ExternalToolCallOutcome::TimedOut => "timed_out",
                                ExternalToolCallOutcome::Cancelled => "cancelled",
                            }
                            .into(),
                            is_error: event.is_error,
                        });
                    }
                    _ => {}
                }
            }
        }
        // A takeover may have happened during storage I/O. Revalidate the lease
        // and prefer any live receipt that arrived meanwhile. Never insert a
        // restored request into the new executor's resolvable pending calls.
        Ok(self.read(connection, lease, request).await?.or(restored))
    }
}

#[cfg(test)]
mod tests;
