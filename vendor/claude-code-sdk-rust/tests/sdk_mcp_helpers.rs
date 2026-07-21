use claude_code_sdk_rust::{
    create_sdk_mcp_server, create_sdk_mcp_server_with_version, tool, tool_with_annotations,
    MCPContent, MCPToolAnnotations,
};
use serde_json::json;

#[test]
fn sdk_mcp_tool_helper_registers_and_calls_tool() {
    let greet = tool(
        "greet",
        "Greet a user",
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }),
        |input| {
            let name = input
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("there");
            Ok(vec![MCPContent::Text {
                text: format!("Hello, {name}!"),
            }])
        },
    );
    let server = create_sdk_mcp_server("people", vec![greet]);

    let tools = server.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "greet");
    assert_eq!(tools[0].description, "Greet a user");

    let result = server.call_tool("greet", json!({"name": "Ada"})).unwrap();
    assert!(matches!(
        &result[0],
        MCPContent::Text { text } if text == "Hello, Ada!"
    ));
}

#[test]
fn sdk_mcp_server_version_is_reported_during_initialize() {
    let server = create_sdk_mcp_server_with_version("people", "2.0.0", Vec::new());
    let servers = std::collections::HashMap::from([("people".to_string(), server)]);

    let response = claude_code_sdk_rust::internal::sdk_mcp::answer_mcp_message(
        &servers,
        "people",
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );

    assert_eq!(response["result"]["serverInfo"]["version"], "2.0.0");
}

#[test]
fn sdk_mcp_tool_helper_accepts_annotations() {
    let inspect = tool_with_annotations(
        "inspect",
        "Inspect project state",
        json!({"type": "object"}),
        MCPToolAnnotations {
            title: Some("Inspect".to_string()),
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
            open_world_hint: false,
            max_result_size_chars: Some(4096),
        },
        |_| {
            Ok(vec![MCPContent::Text {
                text: "ok".to_string(),
            }])
        },
    );
    let server = create_sdk_mcp_server("people", vec![inspect]);
    let servers = std::collections::HashMap::from([("people".to_string(), server)]);

    let response = claude_code_sdk_rust::internal::sdk_mcp::answer_mcp_message(
        &servers,
        "people",
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );

    assert_eq!(
        response["result"]["tools"][0]["annotations"]["title"],
        "Inspect"
    );
    assert_eq!(
        response["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );
    assert_eq!(
        response["result"]["tools"][0]["annotations"]["idempotentHint"],
        true
    );
    assert_eq!(
        response["result"]["tools"][0]["_meta"]["anthropic/maxResultSizeChars"],
        4096
    );
}

#[test]
fn sdk_mcp_tool_errors_are_returned_as_tool_results() {
    let fail = tool("fail", "Always fails", json!({"type": "object"}), |_| {
        Err("Expected error".to_string())
    });
    let server = create_sdk_mcp_server("people", vec![fail]);
    let servers = std::collections::HashMap::from([("people".to_string(), server)]);

    let response = claude_code_sdk_rust::internal::sdk_mcp::answer_mcp_message(
        &servers,
        "people",
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "fail", "arguments": {}}
        }),
    );

    assert_eq!(response["result"]["isError"], true);
    assert_eq!(response["result"]["content"][0]["type"], "text");
    assert_eq!(response["result"]["content"][0]["text"], "Expected error");
}

#[test]
fn sdk_mcp_resource_content_uses_mcp_resource_shape() {
    let readme = tool(
        "readme",
        "Read a resource",
        json!({"type": "object"}),
        |_| {
            Ok(vec![MCPContent::Resource {
                uri: "file:///tmp/readme.md".to_string(),
                mime_type: Some("text/markdown".to_string()),
                text: Some("# Hello".to_string()),
            }])
        },
    );
    let server = create_sdk_mcp_server("people", vec![readme]);
    let servers = std::collections::HashMap::from([("people".to_string(), server)]);

    let response = claude_code_sdk_rust::internal::sdk_mcp::answer_mcp_message(
        &servers,
        "people",
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "readme", "arguments": {}}
        }),
    );

    assert_eq!(response["result"]["content"][0]["type"], "resource");
    assert_eq!(
        response["result"]["content"][0]["resource"]["uri"],
        "file:///tmp/readme.md"
    );
    assert_eq!(
        response["result"]["content"][0]["resource"]["mimeType"],
        "text/markdown"
    );
    assert_eq!(
        response["result"]["content"][0]["resource"]["text"],
        "# Hello"
    );
}

#[test]
fn sdk_mcp_additional_content_types_match_mcp_wire_shapes() {
    let media = tool("media", "Return media", json!({"type": "object"}), |_| {
        Ok(vec![
            MCPContent::Audio {
                data: "abc123".to_string(),
                mime_type: "audio/wav".to_string(),
            },
            MCPContent::ResourceLink {
                uri: "file:///tmp/data.csv".to_string(),
                name: Some("data".to_string()),
                description: Some("CSV data".to_string()),
                mime_type: Some("text/csv".to_string()),
            },
        ])
    });
    let server = create_sdk_mcp_server("people", vec![media]);
    let servers = std::collections::HashMap::from([("people".to_string(), server)]);

    let response = claude_code_sdk_rust::internal::sdk_mcp::answer_mcp_message(
        &servers,
        "people",
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "media", "arguments": {}}
        }),
    );

    assert_eq!(response["result"]["content"][0]["type"], "audio");
    assert_eq!(response["result"]["content"][0]["mimeType"], "audio/wav");
    assert_eq!(response["result"]["content"][1]["type"], "resource_link");
    assert_eq!(
        response["result"]["content"][1]["uri"],
        "file:///tmp/data.csv"
    );
}
