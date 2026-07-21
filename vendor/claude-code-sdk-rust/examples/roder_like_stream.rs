// Mirrors how Roder's claude-code provider drives the SDK: an in-process SDK
// MCP server exposing a read_file tool, built-ins disabled, partial messages.
use claude_code_sdk_rust::mcp::{create_sdk_mcp_server, MCPContent, SdkMcpTool};
use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, PermissionResult, StreamEvent};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tool = SdkMcpTool::new(
        "read_file".to_string(),
        "Read a file from disk".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        }),
        None,
        move |input| {
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| format!("error reading {path}: {err}"));
            Ok(vec![MCPContent::Text { text }])
        },
    );
    let server = create_sdk_mcp_server("roder", vec![tool]);
    let options = ClaudeAgentOptions::builder()
        .model("claude-fable-5")
        .include_partial_messages(true)
        .effort(claude_code_sdk_rust::EffortLevel::High)
        .sdk_mcp_server("roder", server)
        .tools(Vec::new())
        .allowed_tools(vec!["mcp__roder__read_file".to_string()])
        .can_use_tool(|tool_name, _input, _context| async move {
            if tool_name.starts_with("mcp__roder__") {
                Ok(PermissionResult::allow())
            } else {
                Ok(PermissionResult::deny(format!("{tool_name} not managed")))
            }
        })
        .cwd("/tmp/roder-fable-test".to_string())
        .build();

    let mut events = ClaudeAgentClient::spawn_stream_message(
        options,
        "Use the mcp__roder__read_file tool to read README.md and tell me the magic word.",
    );
    while let Some(event) = events.recv().await {
        match event {
            StreamEvent::ContentChunk(text) => print!("{text}"),
            StreamEvent::ToolUseStart { name, .. } => println!("\n[tool start: {name}]"),
            StreamEvent::ToolResult { tool_use_id, .. } => println!("[tool result: {tool_use_id}]"),
            StreamEvent::Complete(response) => {
                println!("\n[complete stop_reason={:?}]", response.stop_reason)
            }
            StreamEvent::Error(message) => println!("\n[ERROR] {message}"),
            _ => {}
        }
    }
}
