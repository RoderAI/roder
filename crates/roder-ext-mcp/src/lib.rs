//! Generic MCP (Model Context Protocol) client extension.
//!
//! Connects to MCP servers over streamable HTTP, discovers their tools at
//! startup, and exposes each remote tool to the agent as
//! `mcp__<server>__<tool>`. Tool calls are forwarded as JSON-RPC
//! `tools/call` requests with either the configured bearer token or, when the
//! server requires it, a thread-scoped bearer token.
//!
//! [`McpStdioClient`] is the stdio transport for local servers that Roder
//! launches itself (for example the browser-use provider in
//! `roder-ext-browser-use`). It owns the server process group and stops it on
//! shutdown or drop.

pub mod client;
pub mod config;
pub mod extension;
pub mod stdio;
pub mod stdio_process;
pub mod tool;

pub use client::{McpHttpClient, McpRpcError, McpToolDescriptor, McpToolOutcome};
pub use config::{McpServerConfig, McpToolCallAuthMode, parse_mcp_servers_json};
pub use extension::McpToolsExtension;
pub use stdio::McpStdioClient;
pub use stdio_process::{McpStdioServerConfig, redact_secrets};
pub use tool::{MCP_TOOL_PROVIDER_ID, McpRemoteTool, McpToolContributor, mcp_tool_name};
