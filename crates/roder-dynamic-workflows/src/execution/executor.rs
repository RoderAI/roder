use std::sync::Arc;

use async_trait::async_trait;
use roder_api::subagents::{SubagentDispatcher, SubagentResult};
use roder_api::trace::SubagentTraceSink;

use super::{WorkflowAgentExecutionContext, WorkflowAgentExecutionRequest};

#[async_trait]
pub trait WorkflowAgentExecutor: Send + Sync + 'static {
    async fn execute_agent(
        &self,
        context: WorkflowAgentExecutionContext,
        request: WorkflowAgentExecutionRequest,
    ) -> anyhow::Result<SubagentResult>;
}

pub struct SubagentDispatcherWorkflowExecutor {
    dispatcher: Arc<dyn SubagentDispatcher>,
    trace_sink: Option<Arc<dyn SubagentTraceSink>>,
}

impl SubagentDispatcherWorkflowExecutor {
    pub fn new(dispatcher: Arc<dyn SubagentDispatcher>) -> Self {
        Self {
            dispatcher,
            trace_sink: None,
        }
    }

    pub fn with_trace_sink(mut self, trace_sink: Arc<dyn SubagentTraceSink>) -> Self {
        self.trace_sink = Some(trace_sink);
        self
    }
}

#[async_trait]
impl WorkflowAgentExecutor for SubagentDispatcherWorkflowExecutor {
    async fn execute_agent(
        &self,
        context: WorkflowAgentExecutionContext,
        request: WorkflowAgentExecutionRequest,
    ) -> anyhow::Result<SubagentResult> {
        let parent_thread = context
            .thread_id
            .clone()
            .unwrap_or_else(|| context.run_id.clone());
        let parent_turn = context
            .turn_id
            .clone()
            .unwrap_or_else(|| context.agent_id.clone());
        self.dispatcher
            .dispatch_traced(
                parent_thread,
                parent_turn,
                request.subagent_request,
                self.trace_sink.clone(),
            )
            .await
    }
}
