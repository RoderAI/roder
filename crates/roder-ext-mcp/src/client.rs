use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::McpServerConfig;

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool metadata returned by `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Option<serde_json::Value>,
}

/// Result of one `tools/call`.
#[derive(Debug, Clone)]
pub struct McpToolOutcome {
    pub text: String,
    pub data: serde_json::Value,
    pub is_error: bool,
}

/// Minimal MCP client over streamable HTTP (stateless-friendly).
///
/// Performs the `initialize` handshake lazily before the first request,
/// tracks the optional `Mcp-Session-Id` header, and accepts both
/// `application/json` and single-message `text/event-stream` responses.
#[derive(Debug, Clone)]
pub struct McpHttpClient {
    config: McpServerConfig,
    http: reqwest::Client,
    next_id: Arc<AtomicI64>,
    session: Arc<Mutex<SessionState>>,
}

#[derive(Debug, Default)]
struct SessionState {
    initialized: bool,
    session_id: Option<String>,
}

impl McpHttpClient {
    pub fn new(config: McpServerConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .context("build MCP HTTP client")?;
        Ok(Self {
            config,
            http,
            next_id: Arc::new(AtomicI64::new(1)),
            session: Arc::new(Mutex::new(SessionState::default())),
        })
    }

    pub fn config(&self) -> &McpServerConfig {
        &self.config
    }

    /// Returns a clone that authenticates with `token` instead of the
    /// configured/env credential. A fresh session is used so the `initialize`
    /// handshake runs under the override identity (the Vex MCP endpoint is
    /// stateless, so re-initializing per scoped call is cheap and correct).
    pub fn with_auth_token_override(&self, token: String) -> Self {
        let mut config = self.config.clone();
        config.auth_token = Some(token);
        Self {
            config,
            http: self.http.clone(),
            next_id: Arc::new(AtomicI64::new(1)),
            session: Arc::new(Mutex::new(SessionState::default())),
        }
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let result = self
            .request("tools/list", serde_json::json!({}))
            .await
            .with_context(|| format!("tools/list against MCP server {}", self.config.name))?;
        let tools = result
            .get("tools")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        let tools: Vec<McpToolDescriptor> =
            serde_json::from_value(tools).context("parse tools/list result")?;
        Ok(tools
            .into_iter()
            .filter(|tool| self.config.tool_enabled(&tool.name))
            .collect())
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<McpToolOutcome> {
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        let result = self.request("tools/call", params).await.with_context(|| {
            format!("tools/call {name} against MCP server {}", self.config.name)
        })?;

        let is_error = result
            .get("isError")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let text = render_content_text(&result);
        let data = result
            .get("structuredContent")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Ok(McpToolOutcome {
            text,
            data,
            is_error,
        })
    }

    /// Sends one JSON-RPC request, running the `initialize` handshake first
    /// if this client has not initialized its session yet.
    async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_initialized().await?;
        self.send_rpc(method, params).await
    }

    async fn ensure_initialized(&self) -> anyhow::Result<()> {
        let mut session = self.session.lock().await;
        if session.initialized {
            return Ok(());
        }

        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "roder-ext-mcp", "version": env!("CARGO_PKG_VERSION") }
        });
        let body = self.rpc_body("initialize", params);
        let response = self.post(&body, session.session_id.as_deref()).await?;
        if let Some(session_id) = header_string(&response, "mcp-session-id") {
            session.session_id = Some(session_id);
        }
        let message = parse_rpc_response(response).await?;
        rpc_result(message)?;

        // Required by the MCP lifecycle; stateless servers reply 202.
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let _ = self.post(&initialized, session.session_id.as_deref()).await;

        session.initialized = true;
        Ok(())
    }

    async fn send_rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let session_id = self.session.lock().await.session_id.clone();
        let body = self.rpc_body(method, params);
        let response = self.post(&body, session_id.as_deref()).await?;
        let message = parse_rpc_response(response).await?;
        rpc_result(message)
    }

    fn rpc_body(&self, method: &str, params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id.fetch_add(1, Ordering::Relaxed),
            "method": method,
            "params": params
        })
    }

    async fn post(
        &self,
        body: &serde_json::Value,
        session_id: Option<&str>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut request = self
            .http
            .post(&self.config.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .json(body);
        if let Some(token) = self.config.resolve_auth_token() {
            request = request.bearer_auth(token);
        }
        if let Some(session_id) = session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        for (key, value) in &self.config.headers {
            request = request.header(key, value);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("POST {}", self.config.url))?;
        let status = response.status();
        if !(status.is_success() || status.as_u16() == 202) {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "MCP server {} returned HTTP {status}: {}",
                self.config.name,
                truncate(&body, 500)
            );
        }
        Ok(response)
    }
}

fn header_string(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Parses an `application/json` or single-message `text/event-stream`
/// JSON-RPC response body.
async fn parse_rpc_response(response: reqwest::Response) -> anyhow::Result<serde_json::Value> {
    let content_type = header_string(&response, "content-type").unwrap_or_default();
    let body = response.text().await.context("read MCP response body")?;
    if content_type.contains("text/event-stream") {
        parse_sse_message(&body)
    } else {
        serde_json::from_str(&body)
            .with_context(|| format!("parse MCP JSON response: {}", truncate(&body, 200)))
    }
}

/// Extracts the last JSON-RPC response message from an SSE body.
fn parse_sse_message(body: &str) -> anyhow::Result<serde_json::Value> {
    let mut last_response = None;
    for event_block in body.split("\n\n") {
        let data = event_block
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if let Ok(message) = serde_json::from_str::<serde_json::Value>(&data)
            && (message.get("result").is_some() || message.get("error").is_some())
        {
            last_response = Some(message);
        }
    }
    last_response.ok_or_else(|| anyhow::anyhow!("no JSON-RPC response found in SSE stream"))
}

fn rpc_result(message: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    if let Some(error) = message.get("error") {
        let code = error
            .get("code")
            .and_then(|code| code.as_i64())
            .unwrap_or(0);
        let text = error
            .get("message")
            .and_then(|text| text.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("MCP error {code}: {text}");
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC response missing result"))
}

/// Joins the `content` text parts of a tool result for the model transcript.
fn render_content_text(result: &serde_json::Value) -> String {
    let Some(content) = result.get("content").and_then(|content| content.as_array()) else {
        return String::new();
    };
    content
        .iter()
        .filter_map(|item| match item.get("type").and_then(|t| t.as_str()) {
            Some("text") => item
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::to_string),
            Some(other) => Some(format!("[unsupported MCP content type: {other}]")),
            None => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &text[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_response_body() {
        let body =
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let message = parse_sse_message(body).unwrap();
        assert_eq!(message["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn renders_text_content() {
        let result = serde_json::json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "image", "data": "...", "mimeType": "image/png" }
            ]
        });
        assert_eq!(
            render_content_text(&result),
            "hello\n[unsupported MCP content type: image]"
        );
    }

    #[test]
    fn rpc_error_is_surfaced() {
        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        });
        let error = rpc_result(message).unwrap_err();
        assert!(error.to_string().contains("-32601"));
    }
}
