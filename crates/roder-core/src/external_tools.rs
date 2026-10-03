//! Metadata readback for host execution delivery recovery; no inputs or outputs.
use crate::Runtime;

#[derive(Debug, Clone)]
pub struct PendingExternalExecution {
    pub request_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub tool_name: String,
}

impl Runtime {
    pub async fn pending_external_executions(&self) -> Vec<PendingExternalExecution> {
        self.pending_external_tool_calls
            .lock()
            .await
            .iter()
            .map(|(id, pending)| PendingExternalExecution {
                request_id: id.clone(),
                thread_id: pending.thread_id.clone(),
                turn_id: pending.turn_id.clone(),
                tool_name: pending.tool_name.clone(),
            })
            .collect()
    }
}
