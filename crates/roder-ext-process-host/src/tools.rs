//! `ToolContributor` adapter backed by a process-hosted child.
//!
//! Tool schemas come statically from the manifest, so contributing to the
//! registry never spawns the child; the first `tools/call` does (lazily),
//! through the same request plumbing and timeouts as inference.

use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::process_extension::{
    METHOD_TOOLS_CALL, ProcessToolCallParams, ProcessToolCallResult,
};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};

use crate::process::ProcessHost;

pub struct ProcessToolContributor {
    host: Arc<ProcessHost>,
    provider_id: String,
    tools: Vec<ToolSpec>,
}

impl ProcessToolContributor {
    pub fn new(host: Arc<ProcessHost>, provider_id: String, tools: Vec<ToolSpec>) -> Self {
        Self {
            host,
            provider_id,
            tools,
        }
    }
}

impl ToolContributor for ProcessToolContributor {
    fn id(&self) -> ToolProviderId {
        self.provider_id.clone()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for spec in &self.tools {
            registry.register(Arc::new(ProcessTool {
                host: self.host.clone(),
                provider_id: self.provider_id.clone(),
                spec: spec.clone(),
            }))?;
        }
        Ok(())
    }
}

struct ProcessTool {
    host: Arc<ProcessHost>,
    provider_id: String,
    spec: ToolSpec,
}

#[async_trait::async_trait]
impl ToolExecutor for ProcessTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let params = ProcessToolCallParams {
            provider_id: self.provider_id.clone(),
            tool_name: self.spec.name.clone(),
            call_id: call.id.clone(),
            thread_id: ctx.thread_id.clone(),
            turn_id: ctx.turn_id.clone(),
            arguments: call.arguments,
        };
        let outcome: anyhow::Result<ProcessToolCallResult> = self
            .host
            .request(METHOD_TOOLS_CALL, serde_json::to_value(&params)?)
            .await;
        // Child JSON-RPC errors become failed tool results — the same shape
        // the runtime wraps native executor errors into — so a broken child
        // fails the call, not the turn.
        Ok(match outcome {
            Ok(result) => ToolResult {
                id: call.id,
                name: call.name,
                text: result.content,
                data: result.data,
                is_error: result.is_error,
            },
            Err(error) => ToolResult {
                id: call.id,
                name: call.name,
                text: error.to_string(),
                data: serde_json::json!({
                    "error": {
                        "kind": "tool_execution_failed",
                        "message": error.to_string(),
                    }
                }),
                is_error: true,
            },
        })
    }
}
