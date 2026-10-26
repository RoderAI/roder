use super::*;
use roder_api::tools::ToolSpec;
use roder_ext_mcp::{
    McpHttpClient, McpServerConfig, McpStdioClient, McpStdioServerConfig, McpToolDescriptor,
    McpToolOutcome,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

#[derive(Default)]
pub(super) struct SessionMcp {
    tools: HashMap<String, BoundTool>,
    clients: Vec<McpClient>,
}

struct BoundTool {
    spec: ToolSpec,
    remote_name: String,
    client: usize,
}
enum McpClient {
    Http(McpHttpClient),
    Stdio(McpStdioClient),
}

impl SessionMcp {
    pub async fn connect(servers: Vec<acp::McpServer>, cwd: &Path) -> Result<Self, JsonRpcError> {
        let mut names = HashSet::new();
        for server in &servers {
            let name = match server {
                acp::McpServer::Http(server) => &server.name,
                acp::McpServer::Stdio(server) => &server.name,
                _ => {
                    return Err(invalid_params(
                        "only stdio and streamable HTTP MCP servers are supported",
                    ));
                }
            };
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                || !names.insert(name.clone())
            {
                return Err(invalid_params(
                    "MCP server names must be unique nonempty identifiers",
                ));
            }
        }
        let mut session = Self::default();
        if let Err(error) = session.connect_servers(servers, cwd).await {
            let _ = session.close().await;
            return Err(error);
        }
        Ok(session)
    }

    async fn connect_servers(
        &mut self,
        servers: Vec<acp::McpServer>,
        cwd: &Path,
    ) -> Result<(), JsonRpcError> {
        for server in servers {
            let (name, client) = match server {
                acp::McpServer::Http(server) => {
                    let mut config = McpServerConfig::new(&server.name, server.url);
                    config.headers = server
                        .headers
                        .into_iter()
                        .map(|header| (header.name, header.value))
                        .collect();
                    (
                        server.name,
                        McpClient::Http(McpHttpClient::new(config).map_err(internal_error)?),
                    )
                }
                acp::McpServer::Stdio(server) => {
                    let mut env = BTreeMap::new();
                    for name in [
                        "PATH",
                        "HOME",
                        "USER",
                        "TMPDIR",
                        "TMP",
                        "TEMP",
                        "LANG",
                        "SystemRoot",
                        "COMSPEC",
                        "PATHEXT",
                    ] {
                        if let Ok(value) = std::env::var(name) {
                            env.insert(name.into(), value);
                        }
                    }
                    let redact = server.env.iter().map(|entry| entry.value.clone()).collect();
                    env.extend(
                        server
                            .env
                            .into_iter()
                            .map(|entry| (entry.name, entry.value)),
                    );
                    let client = McpStdioClient::start(McpStdioServerConfig {
                        name: server.name.clone(),
                        program: server.command,
                        args: server.args,
                        cwd: Some(cwd.to_path_buf()),
                        env,
                        redact,
                        startup_timeout: Duration::from_secs(30),
                    })
                    .await
                    .map_err(internal_error)?;
                    (server.name, McpClient::Stdio(client))
                }
                _ => unreachable!("MCP transport validated before connection"),
            };
            let client_index = self.clients.len();
            self.clients.push(client);
            let descriptors: Vec<McpToolDescriptor> =
                tokio::time::timeout(Duration::from_secs(30), async {
                    match &self.clients[client_index] {
                        McpClient::Http(client) => client.list_tools().await,
                        McpClient::Stdio(client) => {
                            client.list_tools(Duration::from_secs(30)).await
                        }
                    }
                })
                .await
                .map_err(internal_error)?
                .map_err(internal_error)?;
            for descriptor in descriptors {
                if descriptor.name.is_empty()
                    || !descriptor
                        .name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                {
                    return Err(invalid_params("invalid MCP tool name"));
                }
                let spec = ToolSpec {
                    name: roder_ext_mcp::mcp_tool_name(&name, &descriptor.name),
                    description: descriptor
                        .description
                        .unwrap_or_else(|| format!("MCP tool {}", descriptor.name)),
                    parameters: descriptor
                        .input_schema
                        .unwrap_or_else(|| serde_json::json!({"type":"object","properties":{}})),
                };
                if self
                    .tools
                    .insert(
                        spec.name.clone(),
                        BoundTool {
                            spec,
                            remote_name: descriptor.name,
                            client: client_index,
                        },
                    )
                    .is_some()
                {
                    return Err(invalid_params("duplicate MCP tool name"));
                }
            }
        }
        Ok(())
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = self
            .tools
            .values()
            .map(|tool| tool.spec.clone())
            .collect::<Vec<_>>();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub async fn call(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolOutcome, JsonRpcError> {
        if !arguments.is_object() {
            return Err(invalid_params("MCP arguments must be an object"));
        }
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| invalid_params("MCP tool is not bound to this session"))?;
        match &self.clients[tool.client] {
            McpClient::Http(client) => client
                .call_tool(&tool.remote_name, arguments)
                .await
                .map_err(internal_error),
            McpClient::Stdio(client) => client
                .call_tool(&tool.remote_name, arguments, Duration::from_secs(120))
                .await
                .map_err(internal_error),
        }
    }

    pub async fn close(&self) -> Result<(), JsonRpcError> {
        let mut failure = None;
        for client in &self.clients {
            match client {
                McpClient::Stdio(client) => client.shutdown().await,
                McpClient::Http(client) => {
                    if let Err(error) = client.shutdown().await {
                        failure = Some(internal_error(error));
                    }
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }
}

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn handle_mcp_call(
        &self,
        session: &AcpSession,
        notification: &JsonRpcNotification,
        peer: &impl AcpClientPeer,
    ) -> Result<(), JsonRpcError> {
        let request: roder_protocol::ToolExecutionRequestedNotification =
            serde_json::from_value(notification.params.clone()).map_err(internal_error)?;
        let state: roder_protocol::ThreadStateResult = self
            .call(
                "thread/state",
                serde_json::json!({"threadId":session.thread_id}),
            )
            .await?;
        use roder_api::policy_mode::PolicyMode;
        let approved = match state.mode {
            PolicyMode::Plan => false,
            PolicyMode::Bypass => true,
            _ => {
                let call = acp::ToolCallUpdate::new(
                    request.call.id.clone(),
                    acp::ToolCallUpdateFields::new()
                        .title(request.call.name.clone())
                        .kind(acp::ToolKind::Other)
                        .status(acp::ToolCallStatus::Pending)
                        .raw_input(request.call.arguments.clone()),
                );
                let response = peer
                    .request_permission(acp::RequestPermissionRequest::new(
                        session.thread_id.clone(),
                        call,
                        vec![
                            acp::PermissionOption::new(
                                "allow_once",
                                "Allow once",
                                acp::PermissionOptionKind::AllowOnce,
                            ),
                            acp::PermissionOption::new(
                                "reject_once",
                                "Reject once",
                                acp::PermissionOptionKind::RejectOnce,
                            ),
                        ],
                    ))
                    .await
                    .map_err(internal_error)?;
                matches!(response.outcome, acp::RequestPermissionOutcome::Selected(ref choice) if choice.option_id.to_string() == "allow_once")
            }
        };
        let current: roder_protocol::ThreadStateResult = self
            .call(
                "thread/state",
                serde_json::json!({"threadId":session.thread_id}),
            )
            .await?;
        let outcome = if approved && current.mode != PolicyMode::Plan {
            session
                .mcp
                .call(&request.call.name, request.call.arguments)
                .await
        } else {
            Err(invalid_params("MCP tool execution was not permitted"))
        };
        let (output, is_error) = match outcome {
            Ok(outcome) => {
                let output = if outcome.data.is_null() {
                    outcome.text
                } else {
                    format!("{}\n{}", outcome.text, outcome.data)
                };
                (output, outcome.is_error)
            }
            Err(error) => (error.message, true),
        };
        let _: roder_protocol::ToolsResolveResult = self
            .call(
                "tools/resolve",
                serde_json::to_value(roder_protocol::ToolsResolveParams {
                    request_id: request.request_id,
                    output,
                    is_error,
                    executor: request.executor,
                    turn_id: Some(request.turn_id),
                })
                .map_err(internal_error)?,
            )
            .await?;
        Ok(())
    }
}
