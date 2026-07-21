use std::sync::Arc;

use roder_api::extension::ToolProviderId;
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionContext, ToolExecutor, ToolRegistry, ToolResult,
    ToolSpec,
};

use crate::client::{McpHttpClient, McpToolDescriptor};
use crate::config::McpToolCallAuthMode;

pub const MCP_TOOL_PROVIDER_ID: &str = "mcp-tools";

/// Conventional Roder name for an MCP-sourced tool.
pub fn mcp_tool_name(server: &str, remote_tool: &str) -> String {
    format!("mcp__{server}__{remote_tool}")
}

/// Contributes every discovered remote tool to the agent's tool registry.
pub struct McpToolContributor {
    tools: Vec<Arc<McpRemoteTool>>,
}

impl McpToolContributor {
    pub fn new(tools: Vec<Arc<McpRemoteTool>>) -> Self {
        Self { tools }
    }
}

impl ToolContributor for McpToolContributor {
    fn id(&self) -> ToolProviderId {
        MCP_TOOL_PROVIDER_ID.to_string()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for tool in &self.tools {
            registry.register(tool.clone())?;
        }
        Ok(())
    }
}

/// One remote MCP tool proxied through `tools/call`.
pub struct McpRemoteTool {
    client: McpHttpClient,
    descriptor: McpToolDescriptor,
}

impl McpRemoteTool {
    pub fn new(client: McpHttpClient, descriptor: McpToolDescriptor) -> Self {
        Self { client, descriptor }
    }

    pub fn remote_name(&self) -> &str {
        &self.descriptor.name
    }
}

#[async_trait::async_trait]
impl ToolExecutor for McpRemoteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: mcp_tool_name(&self.client.config().name, &self.descriptor.name),
            description: self
                .descriptor
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool {}", self.descriptor.name)),
            parameters: self
                .descriptor
                .input_schema
                .clone()
                .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} })),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let arguments = if call.arguments.is_object() {
            call.arguments.clone()
        } else {
            serde_json::json!({})
        };

        let client = match execution_client(&self.client, &ctx.thread_id) {
            Ok(client) => client,
            Err(error) => {
                return Ok(ToolResult {
                    id: call.id,
                    name: call.name,
                    text: error.to_string(),
                    data: serde_json::Value::Null,
                    is_error: true,
                });
            }
        };

        match client.call_tool(&self.descriptor.name, arguments).await {
            Ok(outcome) => Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: outcome.text,
                data: outcome.data,
                is_error: outcome.is_error,
            }),
            Err(error) => Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: format!("MCP tool call failed: {error:#}"),
                data: serde_json::Value::Null,
                is_error: true,
            }),
        }
    }
}

fn execution_client(client: &McpHttpClient, thread_id: &str) -> anyhow::Result<McpHttpClient> {
    match roder_api::mcp_auth::thread_token(thread_id) {
        Some(token) => Ok(client.with_auth_token_override(token)),
        None if client.config().tool_call_auth_mode
            == McpToolCallAuthMode::ThreadScopedRequired =>
        {
            anyhow::bail!(
                "MCP tool call blocked: server {} requires a thread-scoped authentication token",
                client.config().name
            )
        }
        None => Ok(client.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_follow_mcp_convention() {
        assert_eq!(
            mcp_tool_name("vex", "list_hosted_apps"),
            "mcp__vex__list_hosted_apps"
        );
    }

    #[test]
    fn execution_client_rejects_missing_required_thread_token() {
        let client = McpHttpClient::new(
            crate::config::McpServerConfig::new("vex", "http://127.0.0.1:1/mcp")
                .with_tool_call_auth_mode(McpToolCallAuthMode::ThreadScopedRequired),
        )
        .unwrap();

        let error = execution_client(&client, "thread-without-token").unwrap_err();

        assert!(error.to_string().contains("requires a thread-scoped"));
    }
}
