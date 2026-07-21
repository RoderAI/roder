use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry};
use roder_ext_mcp::{McpServerConfig, McpToolCallAuthMode, McpToolsExtension};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Minimal MCP server speaking stateless streamable HTTP (JSON responses).
/// Asserts bearer auth and counts initialize handshakes.
struct MockMcpServer {
    url: String,
    initialize_count: Arc<AtomicUsize>,
    request_count: Arc<AtomicUsize>,
    authorization_headers: Arc<Mutex<Vec<String>>>,
}

async fn spawn_mock_mcp_server(expected_token: &'static str) -> MockMcpServer {
    spawn_mock_mcp_server_with_tokens(&[expected_token]).await
}

async fn spawn_mock_mcp_server_with_tokens(expected_tokens: &[&str]) -> MockMcpServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let expected_tokens: Vec<String> = expected_tokens
        .iter()
        .map(|token| (*token).to_string())
        .collect();
    let initialize_count = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::new(AtomicUsize::new(0));
    let authorization_headers = Arc::new(Mutex::new(Vec::new()));
    let initialize_counter = initialize_count.clone();
    let request_counter = request_count.clone();
    let recorded_authorization_headers = authorization_headers.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let counter = initialize_counter.clone();
            let request_counter = request_counter.clone();
            let recorded_authorization_headers = recorded_authorization_headers.clone();
            let expected_tokens = expected_tokens.clone();
            tokio::spawn(async move {
                loop {
                    let Some((headers, body)) = read_http_request(&mut stream).await else {
                        break;
                    };
                    request_counter.fetch_add(1, Ordering::SeqCst);
                    let authorization = headers
                        .iter()
                        .find(|header| header.starts_with("authorization:"))
                        .cloned()
                        .unwrap_or_default();
                    recorded_authorization_headers
                        .lock()
                        .unwrap()
                        .push(authorization.clone());
                    let authorized = expected_tokens
                        .iter()
                        .any(|token| authorization == format!("authorization: Bearer {token}"));
                    let response_body = if !authorized {
                        write_response(&mut stream, 401, r#"{"error":"Unauthorized"}"#).await;
                        continue;
                    } else {
                        let message: serde_json::Value =
                            serde_json::from_str(&body).unwrap_or_default();
                        let id = message
                            .get("id")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        match message.get("method").and_then(|m| m.as_str()) {
                            Some("initialize") => {
                                counter.fetch_add(1, Ordering::SeqCst);
                                rpc_result(
                                    id,
                                    serde_json::json!({
                                        "protocolVersion": "2025-06-18",
                                        "capabilities": { "tools": {} },
                                        "serverInfo": { "name": "mock", "version": "0.0.1" }
                                    }),
                                )
                            }
                            Some("notifications/initialized") => {
                                write_response(&mut stream, 202, "").await;
                                continue;
                            }
                            Some("tools/list") => rpc_result(
                                id,
                                serde_json::json!({
                                    "tools": [
                                        {
                                            "name": "list_hosted_apps",
                                            "description": "List hosted apps.",
                                            "inputSchema": { "type": "object", "properties": {} }
                                        },
                                        {
                                            "name": "create_hosted_app",
                                            "description": "Create a hosted app.",
                                            "inputSchema": {
                                                "type": "object",
                                                "properties": { "slug": { "type": "string" } },
                                                "required": ["slug"]
                                            }
                                        }
                                    ]
                                }),
                            ),
                            Some("tools/call") => {
                                let name = message
                                    .pointer("/params/name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or_default()
                                    .to_string();
                                if name == "create_hosted_app" {
                                    rpc_result(
                                        id,
                                        serde_json::json!({
                                            "content": [{ "type": "text", "text": "{\"error\":\"boom\"}" }],
                                            "structuredContent": { "error": "boom" },
                                            "isError": true
                                        }),
                                    )
                                } else {
                                    rpc_result(
                                        id,
                                        serde_json::json!({
                                            "content": [{ "type": "text", "text": "{\"apps\":[]}" }],
                                            "structuredContent": { "apps": [] },
                                            "isError": false
                                        }),
                                    )
                                }
                            }
                            _ => rpc_result(id, serde_json::json!({})),
                        }
                    };
                    write_response(&mut stream, 200, &response_body).await;
                }
            });
        }
    });

    MockMcpServer {
        url: format!("http://{addr}/mcp"),
        initialize_count,
        request_count,
        authorization_headers,
    }
}

fn rpc_result(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<(Vec<String>, String)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buffer, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let headers: Vec<String> = head
        .lines()
        .skip(1)
        .map(|line| line.to_ascii_lowercase().replacen("bearer", "Bearer", 1))
        .collect();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some((headers, String::from_utf8_lossy(&body).to_string()))
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        _ => "Unknown",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn discovers_and_executes_mcp_tools() {
    let server = spawn_mock_mcp_server("test-token").await;

    let config = McpServerConfig::new("vex", server.url).with_auth_token("test-token");
    let extension = McpToolsExtension::discover_async(vec![config])
        .await
        .unwrap();
    assert_eq!(extension.tool_count(), 2);

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    roder_api::extension::RoderExtension::install(&extension, &mut builder).unwrap();
    let registry = builder.build().unwrap();

    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();
    let specs = tools.specs();
    let names: Vec<_> = specs.iter().map(|spec| spec.name.clone()).collect();
    assert!(names.contains(&"mcp__vex__list_hosted_apps".to_string()));
    assert!(names.contains(&"mcp__vex__create_hosted_app".to_string()));

    let ctx = ToolExecutionContext::new("thread-1", "turn-1", PolicyMode::AcceptAll);
    let tool = tools.get("mcp__vex__list_hosted_apps").unwrap();
    let result = tool
        .execute(
            ctx.clone(),
            ToolCall {
                id: "call-1".to_string(),
                name: "mcp__vex__list_hosted_apps".to_string(),
                arguments: serde_json::json!({}),
                raw_arguments: "{}".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.data, serde_json::json!({ "apps": [] }));
    assert!(result.text.contains("apps"));

    let error_tool = tools.get("mcp__vex__create_hosted_app").unwrap();
    let error_result = error_tool
        .execute(
            ctx,
            ToolCall {
                id: "call-2".to_string(),
                name: "mcp__vex__create_hosted_app".to_string(),
                arguments: serde_json::json!({ "slug": "demo" }),
                raw_arguments: "{\"slug\":\"demo\"}".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert!(error_result.is_error);

    // initialize ran once for discovery; tool calls reuse the session.
    assert_eq!(server.initialize_count.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn skips_unreachable_servers() {
    let config = McpServerConfig::new("offline", "http://127.0.0.1:1/mcp");
    let extension = McpToolsExtension::discover_async(vec![config])
        .await
        .unwrap();
    assert_eq!(extension.tool_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_wrong_token() {
    let server = spawn_mock_mcp_server("right-token").await;
    let config = McpServerConfig::new("vex", server.url).with_auth_token("wrong-token");
    let extension = McpToolsExtension::discover_async(vec![config])
        .await
        .unwrap();
    assert_eq!(extension.tool_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn required_thread_auth_blocks_tool_execution_without_http_request() {
    let server = spawn_mock_mcp_server("process-token").await;
    let config = McpServerConfig::new("vex", server.url)
        .with_auth_token("process-token")
        .with_tool_call_auth_mode(McpToolCallAuthMode::ThreadScopedRequired);
    let extension = McpToolsExtension::discover_async(vec![config])
        .await
        .unwrap();
    let requests_after_discovery = server.request_count.load(Ordering::SeqCst);

    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    roder_api::extension::RoderExtension::install(&extension, &mut builder).unwrap();
    let registry = builder.build().unwrap();
    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();
    let thread_id = "thread-required-auth-without-token";
    roder_api::mcp_auth::clear_thread_token(thread_id);
    let result = tools
        .get("mcp__vex__list_hosted_apps")
        .unwrap()
        .execute(
            ToolExecutionContext::new(thread_id, "turn-1", PolicyMode::AcceptAll),
            ToolCall {
                id: "call-required-auth".to_string(),
                name: "mcp__vex__list_hosted_apps".to_string(),
                arguments: serde_json::json!({}),
                raw_arguments: "{}".to_string(),
                thread_id: thread_id.to_string(),
                turn_id: "turn-1".to_string(),
            },
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.text.contains("requires a thread-scoped"));
    assert_eq!(
        server.request_count.load(Ordering::SeqCst),
        requests_after_discovery,
        "missing thread auth must fail before any HTTP request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn required_thread_auth_uses_thread_token_for_tool_execution() {
    let server = spawn_mock_mcp_server_with_tokens(&["process-token", "thread-token"]).await;
    let config = McpServerConfig::new("vex", server.url)
        .with_auth_token("process-token")
        .with_tool_call_auth_mode(McpToolCallAuthMode::ThreadScopedRequired);
    let extension = McpToolsExtension::discover_async(vec![config])
        .await
        .unwrap();
    let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
    roder_api::extension::RoderExtension::install(&extension, &mut builder).unwrap();
    let registry = builder.build().unwrap();
    let mut tools = ToolRegistry::default();
    registry.tools[0].contribute(&mut tools).unwrap();
    let thread_id = "thread-required-auth-with-token";
    roder_api::mcp_auth::set_thread_token(thread_id, "thread-token");

    let result = tools
        .get("mcp__vex__list_hosted_apps")
        .unwrap()
        .execute(
            ToolExecutionContext::new(thread_id, "turn-1", PolicyMode::AcceptAll),
            ToolCall {
                id: "call-required-auth".to_string(),
                name: "mcp__vex__list_hosted_apps".to_string(),
                arguments: serde_json::json!({}),
                raw_arguments: "{}".to_string(),
                thread_id: thread_id.to_string(),
                turn_id: "turn-1".to_string(),
            },
        )
        .await
        .unwrap();
    roder_api::mcp_auth::clear_thread_token(thread_id);

    assert!(!result.is_error);
    assert_eq!(
        server
            .authorization_headers
            .lock()
            .unwrap()
            .last()
            .map(String::as_str),
        Some("authorization: Bearer thread-token")
    );
}
