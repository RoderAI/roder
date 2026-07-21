use std::collections::HashMap;

use crate::mcp::{MCPContent, SimpleMCPServer};
use serde_json::Value;

pub fn answer_mcp_message(
    servers: &HashMap<String, SimpleMCPServer>,
    server_name: &str,
    message: &Value,
) -> Value {
    let Some(server) = servers.get(server_name) else {
        return jsonrpc_error(
            message.get("id").cloned(),
            -32601,
            &format!("Server '{server_name}' not found"),
        );
    };

    let method = message.get("method").and_then(|v| v.as_str()).unwrap_or("");
    match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": server.name(),
                    "version": server.version(),
                },
            },
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "result": {
                "tools": server
                    .list_tools()
                    .into_iter()
                    .map(tool_to_wire)
                    .collect::<Vec<_>>(),
            },
        }),
        "tools/call" => call_tool(server, message),
        "notifications/initialized" => serde_json::json!({
            "jsonrpc": "2.0",
            "result": {},
        }),
        _ => jsonrpc_error(
            message.get("id").cloned(),
            -32601,
            &format!("Method '{method}' not found"),
        ),
    }
}

fn call_tool(server: &SimpleMCPServer, message: &Value) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let params = message.get("params").and_then(|v| v.as_object());
    let name = params
        .and_then(|params| params.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    match server.call_tool(name, arguments) {
        Ok(content) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": content.into_iter().map(content_to_wire).collect::<Vec<_>>(),
            },
        }),
        Err(error) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": error}],
                "isError": true,
            },
        }),
    }
}

fn tool_to_wire(tool: &crate::mcp::MCPTool) -> Value {
    let mut value = serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema,
    });
    if let Some(annotations) = &tool.annotations {
        value["annotations"] = serde_json::json!({
            "title": annotations.title,
            "readOnlyHint": annotations.read_only_hint,
            "destructiveHint": annotations.destructive_hint,
            "idempotentHint": annotations.idempotent_hint,
            "openWorldHint": annotations.open_world_hint,
        });
        if let Some(max_size) = annotations.max_result_size_chars {
            value["_meta"] = serde_json::json!({
                "anthropic/maxResultSizeChars": max_size,
            });
        }
    }
    value
}

fn content_to_wire(content: MCPContent) -> Value {
    match content {
        MCPContent::Text { text } => serde_json::json!({"type": "text", "text": text}),
        MCPContent::Image { data, mime_type } => {
            serde_json::json!({"type": "image", "data": data, "mimeType": mime_type})
        }
        MCPContent::Audio { data, mime_type } => {
            serde_json::json!({"type": "audio", "data": data, "mimeType": mime_type})
        }
        MCPContent::ResourceLink {
            uri,
            name,
            description,
            mime_type,
        } => serde_json::json!({
            "type": "resource_link",
            "uri": uri,
            "name": name,
            "description": description,
            "mimeType": mime_type,
        }),
        MCPContent::Resource {
            uri,
            mime_type,
            text,
        } => serde_json::json!({
            "type": "resource",
            "resource": {
                "uri": uri,
                "mimeType": mime_type,
                "text": text,
            },
        }),
    }
}

fn jsonrpc_error(id: Option<Value>, code: i32, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{MCPContent, MCPTool};

    fn server() -> SimpleMCPServer {
        let mut server = SimpleMCPServer::new("greeter");
        server.register_tool(
            MCPTool {
                name: "greet".to_string(),
                description: "Greet someone".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: None,
            },
            |input| {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("there");
                Ok(vec![MCPContent::Text {
                    text: format!("Hi {name}"),
                }])
            },
        );
        server
    }

    #[test]
    fn answers_tools_list() {
        let servers = HashMap::from([("greeter".to_string(), server())]);
        let response = answer_mcp_message(
            &servers,
            "greeter",
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        assert_eq!(response["result"]["tools"][0]["name"], "greet");
        assert_eq!(
            response["result"]["tools"][0]["inputSchema"]["type"],
            "object"
        );
    }

    #[test]
    fn answers_tools_call() {
        let servers = HashMap::from([("greeter".to_string(), server())]);
        let response = answer_mcp_message(
            &servers,
            "greeter",
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "greet", "arguments": {"name": "Ada"}}
            }),
        );

        assert_eq!(response["result"]["content"][0]["text"], "Hi Ada");
    }
}
