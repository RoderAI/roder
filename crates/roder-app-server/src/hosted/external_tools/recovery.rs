//! Recover execution receipts without replaying browser actions after host loss.
use super::{Call, ExecutorBindings};
use roder_api::events::{ExternalToolCallOutcome, RoderEvent};
use roder_protocol::{ToolExecutionState, ToolExecutorLease};
use std::time::Instant;

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
                                ExternalToolCallOutcome::TimedOut => "timedOut",
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
        // Revalidate under the insertion lock: a takeover may happen during I/O.
        // Cache bounded metadata only, without granting execution ownership.
        let mut bindings = self.inner.lock().await;
        if !bindings.owns(connection, lease) {
            return Err("executor_not_owned".into());
        }
        if let Some(call) = bindings.calls.get(request) {
            return Ok((call.state.thread_id == lease.thread_id).then(|| call.state.clone()));
        }
        if let Some(state) = restored.as_ref() {
            bindings.calls.insert(
                request.into(),
                Call {
                    executor: None,
                    state: state.clone(),
                    updated: Instant::now(),
                },
            );
            bindings.prune();
        }
        Ok(restored)
    }
}

#[cfg(test)]
mod tests;
