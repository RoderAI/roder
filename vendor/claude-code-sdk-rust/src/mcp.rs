//! MCP (Model Context Protocol) support for Claude Agent SDK.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MCPServerConfig {
    /// Stdio-based MCP server.
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// SSE-based MCP server.
    #[serde(rename = "sse")]
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    /// HTTP-based MCP server.
    #[serde(rename = "http")]
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// Information about an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Option<MCPToolAnnotations>,
}

/// Annotations for an MCP tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MCPToolAnnotations {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub read_only_hint: bool,
    #[serde(default)]
    pub destructive_hint: bool,
    #[serde(default)]
    pub idempotent_hint: bool,
    #[serde(default)]
    pub open_world_hint: bool,
    #[serde(default)]
    pub max_result_size_chars: Option<usize>,
}

/// Content block for MCP tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MCPContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "audio")]
    Audio { data: String, mime_type: String },
    #[serde(rename = "resource_link")]
    ResourceLink {
        uri: String,
        name: Option<String>,
        description: Option<String>,
        mime_type: Option<String>,
    },
    #[serde(rename = "resource")]
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
    },
}

/// Status of an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPServerStatus {
    pub name: String,
    pub status: MCPConnectionStatus,
    #[serde(default)]
    pub tools: Vec<MCPTool>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Connection status of an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MCPConnectionStatus {
    Connected,
    Disconnected,
    Error,
}

/// A simple in-process MCP server.
type MCPToolHandler = dyn Fn(Value) -> Result<Vec<MCPContent>, String> + Send + Sync;

/// Definition for an SDK MCP tool.
#[derive(Clone)]
pub struct SdkMcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Option<MCPToolAnnotations>,
    handler: Arc<MCPToolHandler>,
}

impl std::fmt::Debug for SdkMcpTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkMcpTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("input_schema", &self.input_schema)
            .field("annotations", &self.annotations)
            .finish_non_exhaustive()
    }
}

impl SdkMcpTool {
    pub fn new<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        annotations: Option<MCPToolAnnotations>,
        handler: F,
    ) -> Self
    where
        F: Fn(Value) -> Result<Vec<MCPContent>, String> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            annotations,
            handler: Arc::new(handler),
        }
    }

    fn into_parts(self) -> (MCPTool, Arc<MCPToolHandler>) {
        let tool = MCPTool {
            name: self.name,
            description: self.description,
            input_schema: self.input_schema,
            annotations: self.annotations,
        };
        (tool, self.handler)
    }
}

/// Construct an SDK MCP tool definition.
pub fn tool<F>(
    name: impl Into<String>,
    description: impl Into<String>,
    input_schema: Value,
    handler: F,
) -> SdkMcpTool
where
    F: Fn(Value) -> Result<Vec<MCPContent>, String> + Send + Sync + 'static,
{
    SdkMcpTool::new(name, description, input_schema, None, handler)
}

/// Construct an SDK MCP tool definition with MCP annotations.
pub fn tool_with_annotations<F>(
    name: impl Into<String>,
    description: impl Into<String>,
    input_schema: Value,
    annotations: MCPToolAnnotations,
    handler: F,
) -> SdkMcpTool
where
    F: Fn(Value) -> Result<Vec<MCPContent>, String> + Send + Sync + 'static,
{
    SdkMcpTool::new(name, description, input_schema, Some(annotations), handler)
}

#[derive(Clone)]
pub struct SimpleMCPServer {
    name: String,
    version: String,
    tools: HashMap<String, MCPTool>,
    handlers: HashMap<String, Arc<MCPToolHandler>>,
}

impl std::fmt::Debug for SimpleMCPServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleMCPServer")
            .field("name", &self.name)
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl SimpleMCPServer {
    /// Create a new simple MCP server.
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_version(name, "1.0.0")
    }

    /// Create a new simple MCP server with an explicit version.
    pub fn with_version(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            tools: HashMap::new(),
            handlers: HashMap::new(),
        }
    }

    /// Register a tool with the server.
    pub fn register_tool<F>(&mut self, tool: MCPTool, handler: F)
    where
        F: Fn(Value) -> Result<Vec<MCPContent>, String> + Send + Sync + 'static,
    {
        let name = tool.name.clone();
        self.tools.insert(name.clone(), tool);
        self.handlers.insert(name, Arc::new(handler));
    }

    /// Register a pre-built SDK MCP tool with the server.
    pub fn register_sdk_tool(&mut self, tool: SdkMcpTool) {
        let (tool, handler) = tool.into_parts();
        let name = tool.name.clone();
        self.tools.insert(name.clone(), tool);
        self.handlers.insert(name, handler);
    }

    /// Get all registered tools.
    pub fn list_tools(&self) -> Vec<&MCPTool> {
        self.tools.values().collect()
    }

    /// Call a tool by name.
    pub fn call_tool(&self, name: &str, input: Value) -> Result<Vec<MCPContent>, String> {
        if let Some(handler) = self.handlers.get(name) {
            handler(input)
        } else {
            Err(format!("Tool '{}' not found", name))
        }
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the server version.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Initialize an MCP server.
pub fn initialize_server(name: impl Into<String>) -> SimpleMCPServer {
    SimpleMCPServer::new(name)
}

/// Create an in-process SDK MCP server with pre-registered tools.
pub fn create_sdk_mcp_server(name: impl Into<String>, tools: Vec<SdkMcpTool>) -> SimpleMCPServer {
    create_sdk_mcp_server_with_version(name, "1.0.0", tools)
}

/// Create an in-process SDK MCP server with an explicit version.
pub fn create_sdk_mcp_server_with_version(
    name: impl Into<String>,
    version: impl Into<String>,
    tools: Vec<SdkMcpTool>,
) -> SimpleMCPServer {
    let mut server = SimpleMCPServer::with_version(name, version);
    for tool in tools {
        server.register_sdk_tool(tool);
    }
    server
}
