use roder_api::catalog::{PROVIDER_ANTHROPIC, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceProviderContext, InferenceProviderMetadata,
    InferenceTurnContext, MessageDelta, ModelDescriptor, ProviderAuthType, TokenUsage,
    ToolCallCompleted,
};
use roder_api::reliability::{
    ReliabilityRequestPolicy, provider_retry_delay_ms, provider_retry_metadata,
    provider_retry_status_cause,
};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicEngine {
    api_key: String,
}

impl AnthropicEngine {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    fn map_request(request: &AgentInferenceRequest) -> Value {
        let mut body = json!({
            "model": request.model.model,
            "max_tokens": request.output.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "cache_control": { "type": "ephemeral" },
            "messages": anthropic_messages(request),
        });
        let mut system = Vec::new();
        if let Some(text) = request.instructions.system.as_deref() {
            system.push(json!({ "type": "text", "text": text }));
        }
        if let Some(text) = request.instructions.developer.as_deref() {
            system.push(json!({ "type": "text", "text": text }));
        }
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(temperature) = request.output.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(top_p) = request.output.top_p {
            body["top_p"] = json!(top_p);
        }
        if request.reasoning.enabled
            && let Some(level) = request.reasoning.level.as_deref()
        {
            body["output_config"] = json!({ "effort": anthropic_effort(level) });
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        let tool =
                            tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.parameters,
                        })
                    })
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = anthropic_tool_choice(&request.tool_choice);
        }
        body
    }
}

#[async_trait::async_trait]
impl InferenceEngine for AnthropicEngine {
    fn id(&self) -> InferenceEngineId {
        "anthropic".to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: false,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: false,
            image_input: false,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Anthropic".to_string(),
            description: Some("Anthropic API key provider".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".to_string()),
            auth_configured: Some(true),
            recommended: true,
            sort_order: 30,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_ANTHROPIC, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request);
        let (value, retry_events) = send_anthropic_request(
            "https://api.anthropic.com/v1/messages",
            &self.api_key,
            &body,
            request.runtime.reliability.as_ref(),
        )
        .await?;
        let text = extract_message_text(&value);
        let mut events = Vec::new();
        if !text.is_empty() {
            events.push(Ok(InferenceEvent::MessageDelta(MessageDelta {
                text,
                phase: None,
            })));
        }
        for call in extract_tool_calls(&value) {
            events.push(Ok(InferenceEvent::ToolCallCompleted(call)));
        }
        if let Some(usage) = extract_usage(&value) {
            events.push(Ok(InferenceEvent::Usage(usage)));
        }
        for retry_event in retry_events {
            events.push(Ok(InferenceEvent::ProviderMetadata(retry_event)));
        }
        events.push(Ok(InferenceEvent::ProviderMetadata(value.clone())));
        events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: value
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            provider_response_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        })));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

async fn send_anthropic_request(
    url: &str,
    api_key: &str,
    body: &Value,
    policy: Option<&ReliabilityRequestPolicy>,
) -> anyhow::Result<(Value, Vec<Value>)> {
    let policy = policy.cloned().unwrap_or_default();
    let attempts = policy.provider_retry_max_attempts.max(1);
    let client = reqwest::Client::new();
    let mut last_error = None;
    let mut retry_events = Vec::new();
    for attempt in 1..=attempts {
        let response = client
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(body)
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let bytes = response.bytes().await?;
                if bytes.is_empty() && policy.retry_empty_provider_body && attempt < attempts {
                    push_retry_event(&mut retry_events, attempt, "empty_provider_body", &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
                return Ok((serde_json::from_slice(&bytes)?, retry_events));
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let retryable = policy
                    .provider_retry_status_codes
                    .contains(&status.as_u16());
                last_error = Some(format!("Anthropic error {status}: {text}"));
                if retryable && attempt < attempts {
                    push_retry_event(
                        &mut retry_events,
                        attempt,
                        &provider_retry_status_cause(status.as_u16()),
                        &policy,
                    );
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
            Err(err) => {
                last_error = Some(err.to_string());
                if attempt < attempts {
                    push_retry_event(&mut retry_events, attempt, "transport_error", &policy);
                    retry_sleep(&policy, attempt).await;
                    continue;
                }
            }
        }
        break;
    }
    anyhow::bail!(last_error.unwrap_or_else(|| "Anthropic request failed".to_string()))
}

fn push_retry_event(
    events: &mut Vec<Value>,
    attempt: u32,
    cause: &str,
    policy: &ReliabilityRequestPolicy,
) {
    events.push(provider_retry_metadata(attempt, cause, policy));
}

async fn retry_sleep(policy: &ReliabilityRequestPolicy, attempt: u32) {
    let delay = provider_retry_delay_ms(policy, attempt);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

fn anthropic_messages(request: &AgentInferenceRequest) -> Vec<Value> {
    request
        .conversation
        .iter()
        .filter_map(|item| match item {
            roder_api::conversation::ConversationItem::UserMessage(message) => Some(json!({
                "role": "user",
                "content": [{ "type": "text", "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::AssistantMessage(message) => Some(json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::ToolCall(call) => Some(json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": parse_json_object(&call.arguments)
                }]
            })),
            roder_api::conversation::ConversationItem::ToolResult(result) => Some(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": result.id,
                    "content": result.result,
                    "is_error": result.is_error
                }]
            })),
            _ => None,
        })
        .collect()
}

fn anthropic_tool_choice(choice: &roder_api::tools::ToolChoice) -> Value {
    match choice {
        roder_api::tools::ToolChoice::Auto => json!({ "type": "auto" }),
        roder_api::tools::ToolChoice::Any => json!({ "type": "any" }),
        roder_api::tools::ToolChoice::None => json!({ "type": "none" }),
        roder_api::tools::ToolChoice::Specific(name) => {
            json!({ "type": "tool", "name": name })
        }
    }
}

fn anthropic_effort(level: &str) -> &str {
    match level {
        "minimal" => "low",
        "xhigh" => "xhigh",
        level => level,
    }
}

fn parse_json_object(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}))
}

fn extract_message_text(value: &Value) -> String {
    value
        .get("content")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item.get("type").and_then(|v| v.as_str()) {
                    Some("text") | None => item.get("text").and_then(|v| v.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn extract_tool_calls(value: &Value) -> Vec<ToolCallCompleted> {
    value
        .get("content")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("tool_use"))
                .filter_map(|item| {
                    let id = item.get("id").and_then(|v| v.as_str())?.to_string();
                    let name = item.get("name").and_then(|v| v.as_str())?.to_string();
                    let arguments = item
                        .get("input")
                        .map(|input| input.to_string())
                        .unwrap_or_else(|| "{}".to_string());
                    Some(ToolCallCompleted {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt_tokens = number_to_u32(usage.get("input_tokens")).unwrap_or_default();
    let completion_tokens = number_to_u32(usage.get("output_tokens")).unwrap_or_default();
    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
    })
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{
        AssistantMessage, ConversationItem, ToolCallRecord, ToolResultRecord, UserMessage,
    };
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::reliability::ReliabilityRequestPolicy;
    use roder_api::tools::{ToolChoice, ToolSpec};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4-6".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
            },
            conversation: vec![
                ConversationItem::UserMessage(UserMessage::text("Hello")),
                ConversationItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
                ConversationItem::ToolCall(ToolCallRecord {
                    id: "toolu_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"pwd"}"#.to_string(),
                }),
                ConversationItem::ToolResult(ToolResultRecord {
                    id: "toolu_1".to_string(),
                    name: Some("shell".to_string()),
                    result: "ok".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
            ],
            tools: vec![ToolSpec {
                name: "shell".to_string(),
                description: "Run a shell command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(100),
                temperature: Some(0.3),
                top_p: Some(0.8),
                response_format: None,
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_anthropic_request() {
        let body = AnthropicEngine::map_request(&request());
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));
        assert!((body["temperature"].as_f64().unwrap() - 0.3).abs() < 1e-6);
        assert!((body["top_p"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(body["system"][0]["text"], "system");
        assert_eq!(body["system"][1]["text"], "developer");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["input"]["cmd"], "pwd");
        assert_eq!(body["messages"][3]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["name"], "shell");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["tool_choice"]["type"], "auto");
        assert_eq!(body["output_config"]["effort"], "medium");
    }

    #[test]
    fn profile_request_snapshot_maps_anthropic_edit_overlay_and_reasoning() {
        let mut request = request();
        request.instructions.developer = Some(
            "developer\n\n## Model Harness Profile\n\nUse the provided context as the current working set."
                .to_string(),
        );
        request.tools = vec![ToolSpec {
            name: "edit".to_string(),
            description: "Edit a file".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["path", "old_string", "new_string"],
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "additionalProperties": false
            }),
        }];
        request.reasoning.level = Some("low".to_string());
        request.runtime.parallel_tool_calls = Some(false);
        request.metadata = json!({
            "modelProfile": {
                "editTool": "edit",
                "schemaPolicy": "standard_required_first",
                "instructionOverlay": "intuitive_context",
                "parallelToolCalls": false
            }
        });

        let body = AnthropicEngine::map_request(&request);

        assert_eq!(
            body["system"][1]["text"],
            request.instructions.developer.unwrap()
        );
        assert_eq!(body["tools"][0]["name"], "edit");
        assert_eq!(
            body["tools"][0]["input_schema"]["required"],
            json!(["path", "old_string", "new_string"])
        );
        assert_eq!(body["output_config"]["effort"], "low");
        assert!(body.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn normalizes_tool_schema_order_for_anthropic_tools() {
        let mut request = request();
        request.tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run shell command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "additionalProperties": false,
                "required": ["command"]
            }),
        }];

        let body = AnthropicEngine::map_request(&request);
        let schema = serde_json::to_string(&body["tools"][0]["input_schema"]).unwrap();

        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );
    }

    #[test]
    fn anthropic_capabilities_advertise_prompt_cache() {
        let engine = AnthropicEngine::new("test-key".to_string());
        assert!(engine.capabilities().prompt_cache);
    }

    #[test]
    fn extracts_anthropic_text_tool_calls_and_usage() {
        let value = json!({
            "id": "msg_123",
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "tool_use", "id": "toolu_2", "name": "shell", "input": { "cmd": "ls" } },
                { "type": "text", "text": " world" }
            ],
            "usage": { "input_tokens": 11, "output_tokens": 7 }
        });
        assert_eq!(extract_message_text(&value), "hello world");
        assert_eq!(
            extract_tool_calls(&value),
            vec![ToolCallCompleted {
                id: "toolu_2".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }]
        );
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage {
                prompt_tokens: 11,
                completion_tokens: 7,
                total_tokens: 18,
            })
        );
    }

    #[tokio::test]
    async fn retry_recovers_after_retryable_status() {
        let url = spawn_retry_server(vec![
            (429, r#"{"error":"busy"}"#),
            (
                200,
                r#"{"id":"msg_1","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#,
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 2,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![429],
            ..ReliabilityRequestPolicy::default()
        };

        let (value, retry_events) =
            send_anthropic_request(&url, "secret", &json!({}), Some(&policy))
                .await
                .unwrap();

        assert_eq!(extract_message_text(&value), "ok");
        assert_eq!(retry_events[0]["kind"], "reliability_retry_attempt");
        assert_eq!(retry_events[0]["errorClass"], "provider_error");
        assert_eq!(retry_events[0]["decision"], "retry");
        assert_eq!(retry_events[0]["cause"], "status_429");
    }

    #[tokio::test]
    async fn retry_non_retryable_status_fails_once() {
        let (url, request_count) = spawn_counting_retry_server(vec![
            (400, r#"{"error":"bad request"}"#),
            (
                200,
                r#"{"id":"msg_1","content":[{"type":"text","text":"should-not-run"}]}"#,
            ),
        ])
        .await;
        let policy = ReliabilityRequestPolicy {
            provider_retry_max_attempts: 3,
            provider_retry_initial_backoff_ms: 0,
            provider_retry_status_codes: vec![429],
            ..ReliabilityRequestPolicy::default()
        };

        let err = send_anthropic_request(&url, "secret", &json!({}), Some(&policy))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("Anthropic error 400"));
        assert_eq!(request_count.load(Ordering::SeqCst), 1);
    }

    async fn spawn_retry_server(responses: Vec<(u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Retry" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        format!("http://{addr}/v1/messages")
    }

    async fn spawn_counting_retry_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let count = request_count.clone();
        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await.unwrap();
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/v1/messages"), request_count)
    }
}
