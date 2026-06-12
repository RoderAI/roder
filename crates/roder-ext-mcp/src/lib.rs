//! Generic MCP (Model Context Protocol) client extension.
//!
//! Connects to MCP servers over streamable HTTP, discovers their tools at
//! startup, and exposes each remote tool to the agent as
//! `mcp__<server>__<tool>`. Tool calls are forwarded as JSON-RPC
//! `tools/call` requests with the configured bearer token.

pub mod client;
pub mod config;
pub mod extension;
pub mod tool;

pub use client::{McpHttpClient, McpToolDescriptor, McpToolOutcome};
pub use config::{McpServerConfig, parse_mcp_servers_json};
pub use extension::McpToolsExtension;
pub use tool::{MCP_TOOL_PROVIDER_ID, McpRemoteTool, McpToolContributor, mcp_tool_name};
