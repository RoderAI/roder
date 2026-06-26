use std::collections::HashSet;

use futures::StreamExt;
use roder_api::inference::{AgentInferenceRequest, InferenceEventStream};
use roder_api::tools::ToolChoice;
use roder_api::transcript::TranscriptItem;
use serde_json::{Value, json};

use crate::chat_stream::{ChatStreamState, ChatToolNameMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatAuth {
    Bearer(String),
    Header { name: String, value: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MaxTokensField {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsRequestConfig {
    pub provider_name: String,
    pub base_url: String,
    pub auth: ChatAuth,
    pub headers: Vec<(String, String)>,
    pub max_tokens_field: MaxTokensField,
    pub thinking_disabled: bool,
    /// When false, omit `stream_options` (some providers reject it on managed APIs).
    pub include_stream_usage: bool,
    /// When false, omit `parallel_tool_calls` even when tools are present.
    pub include_parallel_tool_calls: bool,
}

impl ChatCompletionsRequestConfig {
    pub fn bearer(
        provider_name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            base_url: base_url.into(),
            auth: ChatAuth::Bearer(api_key.into()),
            headers: Vec::new(),
            max_tokens_field: MaxTokensField::MaxTokens,
            thinking_disabled: false,
            include_stream_usage: true,
            include_parallel_tool_calls: true,
        }
    }

    pub fn api_key_header(
        provider_name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            provider_name: provider_name.into(),
            base_url: base_url.into(),
            auth: ChatAuth::Header {
                name: "api-key".to_string(),
                value: api_key.into(),
            },
            headers: Vec::new(),
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            thinking_disabled: true,
            include_stream_usage: true,
            include_parallel_tool_calls: true,
        }
    }
}

pub async fn stream_chat_completions(
    config: ChatCompletionsRequestConfig,
    request: AgentInferenceRequest,
) -> anyhow::Result<InferenceEventStream> {
    let (tools, tool_name_map) = chat_tools(&request);
    let mut body = json!({
        "model": request.model.model,
        "messages": chat_messages(&request, &tool_name_map),
        "stream": true,
    });
    if config.include_stream_usage {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if config.thinking_disabled {
        body["thinking"] = json!({ "type": "disabled" });
    }
    if request.reasoning.enabled {
        if let Some(level) = request.reasoning.level.as_deref() {
            body["reasoning_effort"] = json!(level);
        }
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = chat_tool_choice(&request.tool_choice, &tool_name_map);
        if config.include_parallel_tool_calls {
            body["parallel_tool_calls"] =
                json!(request.runtime.parallel_tool_calls.unwrap_or(true));
        }
    }
    if let Some(max_tokens) = request.output.max_tokens {
        match config.max_tokens_field {
            MaxTokensField::MaxTokens => body["max_tokens"] = json!(max_tokens),
            MaxTokensField::MaxCompletionTokens => {
                body["max_completion_tokens"] = json!(max_tokens)
            }
        }
    }
    if let Some(temperature) = request.output.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.output.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(response_format) = request.output.response_format {
        body["response_format"] = response_format;
    }

    let client = reqwest::Client::new();
    let mut http = client
        .post(format!(
            "{}/chat/completions",
            config.base_url.trim_end_matches('/')
        ))
        .json(&body);
    http = match &config.auth {
        ChatAuth::Bearer(value) => http.bearer_auth(value),
        ChatAuth::Header { name, value } => http.header(name, value),
    };
    for (key, value) in config.headers {
        http = http.header(key, value);
    }
    let response = http.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(provider_status_error_with_body(
            &config.provider_name,
            "Chat Completions",
            status,
            &body,
            config_auth_secret(&config.auth),
        ));
    }

    let mut bytes = response.bytes_stream();
    let stream = async_stream::try_stream! {
        let mut state = ChatStreamState::new(tool_name_map);
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            for event in state.push_chunk(&chunk)? {
                yield event;
            }
        }
        for event in state.finish() {
            yield event;
        }
    };

    Ok(Box::pin(stream))
}

#[allow(dead_code)]
pub fn redacted_provider_status_error(
    provider_name: &str,
    operation: &str,
    status: reqwest::StatusCode,
) -> anyhow::Error {
    anyhow::anyhow!(
        "{provider_name} {operation} error {status}: {}",
        provider_status_hint(status)
    )
}

/// Build a provider error that includes the response body for the TUI detail
/// popup. The auth credential (bearer token or header value) is scrubbed from
/// the body so it is never surfaced in the timeline or popup.
pub fn provider_status_error_with_body(
    provider_name: &str,
    operation: &str,
    status: reqwest::StatusCode,
    body: &str,
    auth_secret: Option<&str>,
) -> anyhow::Error {
    let hint = provider_status_hint(status);
    let scrubbed = scrub_secret(body, auth_secret);
    let body_part = if scrubbed.trim().is_empty() {
        "(empty response body)".to_string()
    } else {
        scrubbed
    };
    anyhow::anyhow!(
        "{provider_name} {operation} error {status}: {hint}\n\n--- response body ---\n{body_part}"
    )
}

/// Extract the secret value from a [`ChatAuth`] for scrubbing.
fn config_auth_secret(auth: &ChatAuth) -> Option<&str> {
    match auth {
        ChatAuth::Bearer(token) => Some(token),
        ChatAuth::Header { value, .. } => Some(value),
    }
}

/// Remove the auth secret from the body text so it is never leaked into the
/// error message. Also trims the result to a reasonable length.
fn scrub_secret(body: &str, secret: Option<&str>) -> String {
    let trimmed = body.trim();
    let max = 4096;
    let mut result = if trimmed.len() <= max {
        trimmed.to_string()
    } else {
        format!("{}…(truncated, {total} bytes total)", &trimmed[..max], total = trimmed.len())
    };
    if let Some(secret) = secret
        && !secret.is_empty()
    {
        result = result.replace(secret, "<redacted>");
    }
    result
}

fn provider_status_hint(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "invalid request, model, or provider configuration",
        401 | 403 => "authentication or permission failed",
        404 => "endpoint or model not found",
        429 => "rate limited or quota exhausted",
        code if (500..600).contains(&code) => "provider server error",
        _ => "provider returned a non-success response",
    }
}

fn chat_tools(request: &AgentInferenceRequest) -> (Vec<Value>, ChatToolNameMap) {
    let mut tools = Vec::new();
    let mut used_tool_names = HashSet::new();
    let mut tool_name_map = ChatToolNameMap::default();
    for tool in &request.tools {
        let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
        let api_name = chat_tool_name(&tool.name, &mut used_tool_names);
        tool_name_map.register(&tool.name, &api_name);
        tools.push(json!({
            "type": "function",
            "function": {
                "name": api_name,
                "description": tool.description,
                "parameters": tool.parameters,
            },
        }));
    }
    (tools, tool_name_map)
}

fn chat_tool_name(tool_name: &str, used_tool_names: &mut HashSet<String>) -> String {
    let base_name = tool_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if used_tool_names.insert(base_name.clone()) {
        return base_name;
    }
    for suffix in 2u32.. {
        let candidate = format!("{base_name}_{suffix}");
        if used_tool_names.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn chat_tool_choice(choice: &ToolChoice, tool_name_map: &ChatToolNameMap) -> Value {
    match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Any => json!("required"),
        ToolChoice::None => json!("none"),
        ToolChoice::Specific(name) => json!({
            "type": "function",
            "function": { "name": tool_name_map.api_name(name) },
        }),
    }
}

fn chat_messages(request: &AgentInferenceRequest, tool_name_map: &ChatToolNameMap) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.instructions.system {
        messages.push(json!({ "role": "system", "content": system }));
    }
    if let Some(developer) = &request.instructions.developer {
        messages.push(json!({ "role": "system", "content": developer }));
    }
    // The chat-completions API requires that a single assistant message carry
    // ALL of a turn's `tool_calls`, immediately followed by one `role: tool`
    // message per id. The model emits parallel tool calls (e.g. several
    // `write_file` calls at once) as consecutive `ToolCall` transcript items, so
    // we coalesce a run of consecutive `ToolCall`s into one assistant message.
    // Emitting each as its own assistant message produces an invalid request
    // ("tool_call_ids did not have response messages").
    let mut pending_tool_calls: Vec<Value> = Vec::new();
    for item in &request.transcript {
        if let TranscriptItem::ToolCall(call) = item {
            pending_tool_calls.push(json!({
                "id": call.id,
                "type": "function",
                "function": {
                    "name": tool_name_map.api_name(&call.name),
                    "arguments": call.arguments,
                },
            }));
            continue;
        }
        flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
        match item {
            TranscriptItem::UserMessage(message) => {
                messages.push(json!({ "role": "user", "content": message.text }));
            }
            TranscriptItem::AssistantMessage(message) => {
                if !message.text.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": message.text }));
                }
            }
            TranscriptItem::ToolResult(result) => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": result.id,
                    "content": result.result,
                }));
            }
            TranscriptItem::ContextCompaction(compaction) => {
                messages.push(json!({ "role": "system", "content": compaction.summary }));
            }
            TranscriptItem::ReasoningSummary(summary) => {
                messages.push(json!({ "role": "assistant", "content": summary.text }));
            }
            TranscriptItem::ToolCall(_)
            | TranscriptItem::FileChange(_)
            | TranscriptItem::Error(_)
            | TranscriptItem::ProviderMetadata(_) => {}
        }
    }
    flush_pending_tool_calls(&mut messages, &mut pending_tool_calls);
    messages
}

/// Emit any buffered parallel tool calls as a single assistant `tool_calls`
/// message, then clear the buffer. No-op when there is nothing pending.
fn flush_pending_tool_calls(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": std::mem::take(pending),
    }));
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::ToolSpec;
    use roder_api::transcript::{TranscriptItem, UserMessage};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn parallel_tool_calls_coalesce_into_one_assistant_message() {
        use roder_api::transcript::{AssistantMessage, ToolCallRecord, ToolResultRecord};

        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "kimi-code".to_string(),
                model: "kimi-for-coding".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("make a calculator")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "On it.".to_string(),
                    phase: None,
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "write_file:0".to_string(),
                    name: "write_file".to_string(),
                    arguments: "{\"path\":\"a.swift\"}".to_string(),
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "write_file:1".to_string(),
                    name: "write_file".to_string(),
                    arguments: "{\"path\":\"b.swift\"}".to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "write_file:0".to_string(),
                    name: Some("write_file".to_string()),
                    result: "wrote a.swift".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "write_file:1".to_string(),
                    name: Some("write_file".to_string()),
                    result: "wrote b.swift".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
            ],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };

        let messages = chat_messages(&request, &ChatToolNameMap::default());

        // user, assistant text, ONE assistant tool_calls message (both ids),
        // then the two tool results — not two separate tool_calls messages.
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(roles, ["user", "assistant", "assistant", "tool", "tool"]);

        let tool_call_message = &messages[2];
        let calls = tool_call_message["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2, "both parallel calls share one message");
        assert_eq!(calls[0]["id"], "write_file:0");
        assert_eq!(calls[1]["id"], "write_file:1");
        assert_eq!(messages[3]["tool_call_id"], "write_file:0");
        assert_eq!(messages[4]["tool_call_id"], "write_file:1");

        // Interleaved call/result pairs stay valid too (one call per message).
        let interleaved = AgentInferenceRequest {
            transcript: vec![
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "a".to_string(),
                    name: "read_file".to_string(),
                    arguments: "{}".to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "a".to_string(),
                    name: None,
                    result: "ok".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "b".to_string(),
                    name: "read_file".to_string(),
                    arguments: "{}".to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "b".to_string(),
                    name: None,
                    result: "ok".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
            ],
            ..request
        };
        let messages = chat_messages(&interleaved, &ChatToolNameMap::default());
        let roles: Vec<&str> = messages
            .iter()
            .map(|m| m["role"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(roles, ["assistant", "tool", "assistant", "tool"]);
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stream_chat_completions_uses_api_key_header_and_max_completion_tokens() {
        let server = spawn_chat_server(
            "/chat/completions",
            "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\r\n\r\ndata: [DONE]\r\n\r\n",
        )
        .await;
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "xiaomi-mimo".to_string(),
                model: "mimo-v2.5-pro".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
            tools: vec![ToolSpec {
                name: "run command".to_string(),
                description: "Run a command".to_string(),
                parameters: json!({ "type": "object", "properties": {} }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig {
                max_tokens: Some(32),
                ..OutputConfig::default()
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };

        let mut stream = stream_chat_completions(
            ChatCompletionsRequestConfig::api_key_header(
                "Xiaomi MiMo",
                server.base_url.clone(),
                "---",
            ),
            request,
        )
        .await
        .unwrap();
        while stream.next().await.is_some() {}
        let (headers, body) = server.request.await.unwrap();

        assert!(headers.iter().any(|line| line == "api-key: ---"));
        assert_eq!(body["max_completion_tokens"], 32);
        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert_eq!(body["tools"][0]["function"]["name"], "run_command");
    }

    #[tokio::test]
    async fn stream_chat_completions_can_omit_stream_options_and_parallel_tool_calls() {
        let server = spawn_chat_server(
            "/chat/completions",
            "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
        )
        .await;
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "kimi-code".to_string(),
                model: "kimi-for-coding".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
            tools: vec![ToolSpec {
                name: "read_file".to_string(),
                description: "Read".to_string(),
                parameters: json!({ "type": "object", "properties": {} }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                parallel_tool_calls: Some(true),
                ..RuntimeHints::default()
            },
            metadata: json!({}),
        };

        let mut config =
            ChatCompletionsRequestConfig::bearer("Kimi Code", server.base_url.clone(), "token");
        config.include_stream_usage = false;
        config.include_parallel_tool_calls = false;

        let mut stream = stream_chat_completions(config, request).await.unwrap();
        while stream.next().await.is_some() {}
        let (_, body) = server.request.await.unwrap();

        assert!(body.get("stream_options").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[tokio::test]
    async fn stream_chat_completions_includes_but_scrubs_error_response_body() {
        let base_url = spawn_error_server(
            "HTTP/1.1 401 Unauthorized",
            "{\"error\":\"bad api-key tp-secret should not appear\"}",
        )
        .await;
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: "xiaomi-mimo-token-plan".to_string(),
                model: "mimo-v2.5-pro".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };

        let error = match stream_chat_completions(
            ChatCompletionsRequestConfig::api_key_header("Xiaomi MiMo", base_url, "tp-secret"),
            request,
        )
        .await
        {
            Ok(_) => panic!("expected chat completions error"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("401 Unauthorized"));
        assert!(error.contains("authentication or permission failed"));
        // The auth credential must be scrubbed from the body.
        assert!(!error.contains("tp-secret"), "leaked key: {error}");
        // The response body is now included in the error (for the TUI popup).
        assert!(
            error.contains("bad api-key") || error.contains("<redacted>"),
            "body should be included: {error}"
        );
    }

    struct CapturedChatServer {
        base_url: String,
        request: tokio::sync::oneshot::Receiver<(Vec<String>, Value)>,
    }

    async fn spawn_chat_server(
        expected_path: &'static str,
        response_body: &'static str,
    ) -> CapturedChatServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 16 * 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            assert_eq!(path, expected_path);
            let headers = request
                .lines()
                .skip(1)
                .take_while(|line| !line.is_empty())
                .map(|line| line.to_string())
                .collect::<Vec<_>>();
            let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
            tx.send((headers, serde_json::from_str(body).unwrap()))
                .unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        CapturedChatServer {
            base_url: format!("http://{addr}"),
            request: rx,
        }
    }

    async fn spawn_error_server(status_line: &'static str, response_body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 16 * 1024];
            let _ = stream.read(&mut buf).await.unwrap();
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}")
    }
}
