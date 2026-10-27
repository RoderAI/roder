use std::sync::Arc;

use futures::future::try_join_all;
use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::ToolCallCompleted;
use roder_api::tools::ToolExecutionMode;
use roder_api::transcript::ToolResultRecord;
use time::OffsetDateTime;

use crate::runtime::Runtime;

/// Coordinators await nested tools or turns. They cannot hold the resource
/// barrier while doing so; the concrete child operations acquire it instead.
pub(crate) fn is_coordinator(name: &str) -> bool {
    crate::agent_control_tools::is_agent_control_tool(name)
        || crate::thread_events::is_event_tool(name)
}

impl Runtime {
    pub(crate) fn tool_execution_mode(&self, name: &str) -> ToolExecutionMode {
        self.tool_registry
            .get(name)
            .map_or(ToolExecutionMode::Exclusive, |tool| tool.execution_mode())
    }

    pub(crate) async fn execute_scheduled_tools(
        self: &Arc<Self>,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        calls: Vec<ToolCallCompleted>,
        parallel: bool,
        workspace: Option<&str>,
        deadline: Option<OffsetDateTime>,
    ) -> anyhow::Result<Vec<ToolResultRecord>> {
        let parallel = parallel
            && !calls.iter().any(|call| {
                is_coordinator(&call.name)
                    || self.tool_execution_mode(&call.name) == ToolExecutionMode::Coordinator
            });
        let mut calls = calls.into_iter().peekable();
        let mut results = Vec::new();
        while let Some(call) = calls.next() {
            if !parallel || self.tool_execution_mode(&call.name) != ToolExecutionMode::SharedRead {
                results.push(
                    self.route_tool_call(thread_id, turn_id, call, workspace, deadline, None)
                        .await?,
                );
                continue;
            }

            // Only adjacent shared reads form a batch. A mutation is a barrier
            // in response order, even when its approval or provisioning waits.
            let mut reads = vec![call];
            while calls.peek().is_some_and(|call| {
                self.tool_execution_mode(&call.name) == ToolExecutionMode::SharedRead
            }) {
                reads.push(calls.next().expect("peeked read"));
            }
            results.extend(
                try_join_all(reads.into_iter().map(|call| {
                    self.route_tool_call(thread_id, turn_id, call, workspace, deadline, None)
                }))
                .await?,
            );
        }
        Ok(results)
    }
}

#[cfg(test)]
#[path = "tool_scheduling/tests.rs"]
mod tests;
