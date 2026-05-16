use roder_api::catalog::{PROVIDER_OPENAI, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceCapabilities, InferenceEngine,
    InferenceEvent, InferenceEventStream, InferenceFailure, InferenceProviderContext,
    InferenceProviderMetadata, InferenceTurnContext, MessageDelta, ModelDescriptor,
    ProviderAuthType, ReasoningDelta, TokenUsage, ToolCallCompleted, ToolCallDelta,
    ToolCallStarted,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const FINAL_ANSWER_PHASE: &str = "final_answer";

pub struct OpenAiResponsesEngine {
    api_key: String,
    provider_id: String,
    base_url: String,
    headers: Vec<(String, String)>,
}

impl OpenAiResponsesEngine {
    pub fn new(api_key: String) -> Self {
        Self::new_with_provider_id(api_key, PROVIDER_OPENAI)
    }

    pub fn new_with_provider_id(api_key: String, provider_id: impl Into<String>) -> Self {
        Self::new_with_config(
            api_key,
            provider_id,
            "https://api.openai.com/v1",
            Vec::new(),
        )
    }

    pub fn new_with_config(
        api_key: String,
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            api_key,
            provider_id: provider_id.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            headers,
        }
    }

    fn map_request(request: &AgentInferenceRequest) -> Value {
        let mut body = json!({
            "model": request.model.model,
            "input": response_input_items(request),
            "store": false,
            "stream": true,
        });
        if let Some(system) = request
            .instructions
            .system
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            body["instructions"] = json!(system);
        }
        if let Some(max_tokens) = request.output.max_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = request.output.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(top_p) = request.output.top_p {
            body["top_p"] = json!(top_p);
        }
        if let Some(format) = request.output.response_format.as_ref() {
            body["text"] = json!({ "format": format });
        }
        if request.reasoning.enabled {
            body["reasoning"] = match request.reasoning.level.as_deref() {
                Some(level) => json!({ "effort": level }),
                None => json!({}),
            };
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = match &request.tool_choice {
                roder_api::tools::ToolChoice::None => json!("none"),
                roder_api::tools::ToolChoice::Specific(name) => {
                    json!({ "type": "function", "name": name })
                }
                roder_api::tools::ToolChoice::Auto | roder_api::tools::ToolChoice::Any => {
                    json!("auto")
                }
            };
        }
        if let Some(prompt_cache_key) = request.runtime.prompt_cache_key.as_deref() {
            body["prompt_cache_key"] = json!(prompt_cache_key);
        }
        body
    }
}

#[async_trait::async_trait]
impl InferenceEngine for OpenAiResponsesEngine {
    fn id(&self) -> InferenceEngineId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: true,
            structured_output: true,
            image_input: true,
            prompt_cache: true,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "OpenAI".to_string(),
            description: Some("OpenAI API key provider".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("API key".to_string()),
            recommended: true,
            sort_order: 20,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(PROVIDER_OPENAI, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = Self::map_request(&request);
        let client = reqwest::Client::new();
        let mut request = client
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(&self.api_key);
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "OpenAI Responses error {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        Ok(stream_responses_sse(response))
    }
}

fn stream_responses_sse(response: reqwest::Response) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        use futures::StreamExt as _;

        let mut chunks = response.bytes_stream();
        let mut buffer = String::new();
        let mut state = ResponsesStreamState::default();

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some((frame, consumed)) = take_sse_frame(&buffer) {
                buffer.drain(..consumed);
                let Some(event) = parse_sse_frame(&frame)? else {
                    continue;
                };
                for inference_event in events_from_sse_event(&event, &mut state) {
                    yield inference_event;
                }
            }
        }

        let trailing_event = if buffer.trim().is_empty() {
            None
        } else {
            parse_sse_frame(&buffer)?
        };
        if let Some(event) = trailing_event {
            for inference_event in events_from_sse_event(&event, &mut state) {
                yield inference_event;
            }
        }

        if !state.terminal {
            Err(anyhow::anyhow!("stream closed before response.completed"))?;
        }
    })
}

#[derive(Default)]
struct ResponsesStreamState {
    terminal: bool,
    streamed_final_text: bool,
    current_message_phase: String,
    message_phases: HashMap<String, String>,
    tool_arguments: HashMap<String, String>,
    tool_names: HashMap<String, String>,
    tool_call_ids: HashMap<String, String>,
    emitted_tool_call_ids: HashSet<String>,
    reasoning_delta_keys: HashSet<String>,
}

#[derive(Debug, PartialEq)]
struct SseEvent {
    event: Option<String>,
    data: Value,
}

fn take_sse_frame(buffer: &str) -> Option<(String, usize)> {
    let lf = buffer.find("\n\n").map(|idx| (idx, 2));
    let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
    let (idx, delimiter_len) = match (lf, crlf) {
        (Some(lf), Some(crlf)) => lf.min(crlf),
        (Some(lf), None) => lf,
        (None, Some(crlf)) => crlf,
        (None, None) => return None,
    };
    Some((buffer[..idx].to_string(), idx + delimiter_len))
}

fn parse_sse_frame(frame: &str) -> anyhow::Result<Option<SseEvent>> {
    let mut event = None;
    let mut data = Vec::new();

    for raw_line in frame.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start());
        }
    }

    if data.is_empty() {
        return Ok(None);
    }

    let data = data.join("\n");
    if data.trim() == "[DONE]" {
        return Ok(None);
    }

    Ok(Some(SseEvent {
        event,
        data: serde_json::from_str(&data)?,
    }))
}

fn events_from_sse_event(
    event: &SseEvent,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let kind = event
        .data
        .get("type")
        .and_then(|value| value.as_str())
        .or(event.event.as_deref())
        .unwrap_or_default();

    match kind {
        "response.output_text.delta" => {
            let phase = output_text_phase(&event.data, state);
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    if is_final_answer_phase(&phase) {
                        state.streamed_final_text = true;
                    }
                    InferenceEvent::MessageDelta(MessageDelta {
                        text: text.to_string(),
                        phase: (!phase.is_empty()).then_some(phase),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(key) = reasoning_content_key(kind, &event.data) {
                state.reasoning_delta_keys.insert(key);
            }
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
            let key = reasoning_content_key(kind, &event.data);
            if key
                .as_ref()
                .is_some_and(|key| state.reasoning_delta_keys.contains(key))
            {
                return Vec::new();
            }
            event
                .data
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.output_item.added" => {
            if let Some(item) = event.data.get("item") {
                record_output_item(item, state);
                if let Some(call) = started_function_call(item, state) {
                    return vec![InferenceEvent::ToolCallStarted(call)];
                }
            }
            Vec::new()
        }
        "response.function_call_arguments.delta" => {
            if let Some(item_id) = event.data.get("item_id").and_then(Value::as_str)
                && let Some(delta) = event.data.get("delta").and_then(Value::as_str)
            {
                state
                    .tool_arguments
                    .entry(item_id.to_string())
                    .or_default()
                    .push_str(delta);
                let id = state
                    .tool_call_ids
                    .get(item_id)
                    .cloned()
                    .unwrap_or_else(|| item_id.to_string());
                return vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id,
                    arguments_delta: delta.to_string(),
                })];
            }
            Vec::new()
        }
        "response.function_call_arguments.done" => finalized_function_call(&event.data, state)
            .and_then(|call| emit_tool_call_once(call, state))
            .into_iter()
            .collect(),
        "response.output_item.done" => {
            let calls = event
                .data
                .get("item")
                .map(|item| {
                    record_output_item(item, state);
                    extract_tool_calls_from_item(item)
                })
                .unwrap_or_default();
            calls
                .into_iter()
                .filter_map(|call| emit_tool_call_once(call, state))
                .collect()
        }
        "response.completed" => {
            state.terminal = true;
            let response = event.data.get("response").unwrap_or(&event.data);
            let mut events = Vec::new();
            let text = extract_response_text(response);
            if !state.streamed_final_text && !text.is_empty() {
                events.push(InferenceEvent::MessageDelta(MessageDelta {
                    text,
                    phase: Some(FINAL_ANSWER_PHASE.to_string()),
                }));
            }
            for call in extract_tool_calls(response) {
                if let Some(call) = emit_tool_call_once(call, state) {
                    events.push(call);
                }
            }
            if let Some(usage) = extract_usage(response) {
                events.push(InferenceEvent::Usage(usage));
            }
            events.push(InferenceEvent::ProviderMetadata(response.clone()));
            events.push(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: response
                    .get("status")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                provider_response_id: response
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }));
            events
        }
        "response.failed" | "response.incomplete" | "error" => {
            state.terminal = true;
            vec![InferenceEvent::Failed(InferenceFailure {
                message: stream_error_message(&event.data, kind),
            })]
        }
        _ => Vec::new(),
    }
}

fn reasoning_content_key(kind: &str, data: &Value) -> Option<String> {
    let item_id = data.get("item_id").and_then(Value::as_str)?;
    let content_index = data
        .get("content_index")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let kind = kind
        .strip_suffix(".delta")
        .or_else(|| kind.strip_suffix(".done"))
        .unwrap_or(kind);
    Some(format!("{kind}:{item_id}:{content_index}"))
}

fn output_text_phase(data: &Value, state: &ResponsesStreamState) -> String {
    data.get("item_id")
        .and_then(Value::as_str)
        .and_then(|item_id| state.message_phases.get(item_id))
        .cloned()
        .unwrap_or_else(|| state.current_message_phase.clone())
}

fn is_final_answer_phase(phase: &str) -> bool {
    phase.is_empty() || phase == FINAL_ANSWER_PHASE
}

fn record_output_item(item: &Value, state: &mut ResponsesStreamState) {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => {
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            state.current_message_phase = phase.clone();
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                state.message_phases.insert(id.to_string(), phase);
            }
        }
        Some("function_call") => {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                state.tool_names.insert(id.to_string(), name.to_string());
            }
            if let Some(call_id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                state
                    .tool_call_ids
                    .insert(id.to_string(), call_id.to_string());
            }
            if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                state
                    .tool_arguments
                    .insert(id.to_string(), arguments.to_string());
            }
        }
        _ => {}
    }
}

fn started_function_call(item: &Value, state: &ResponsesStreamState) -> Option<ToolCallStarted> {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| state.tool_names.get(item_id).cloned())?;
    Some(ToolCallStarted { id, name })
}

fn finalized_function_call(
    data: &Value,
    state: &mut ResponsesStreamState,
) -> Option<ToolCallCompleted> {
    let item_id = data.get("item_id").and_then(Value::as_str)?;
    let arguments = data
        .get("arguments")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            state
                .tool_arguments
                .get(item_id)
                .cloned()
                .unwrap_or_else(|| "{}".to_string())
        });
    state
        .tool_arguments
        .insert(item_id.to_string(), arguments.clone());
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| state.tool_names.get(item_id).cloned())?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());

    Some(ToolCallCompleted {
        id,
        name,
        arguments,
    })
}

fn emit_tool_call_once(
    call: ToolCallCompleted,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    state
        .emitted_tool_call_ids
        .insert(call.id.clone())
        .then_some(InferenceEvent::ToolCallCompleted(call))
}

fn extract_tool_calls_from_item(item: &Value) -> Vec<ToolCallCompleted> {
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return Vec::new();
    }

    let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|value| value.as_str())
    else {
        return Vec::new();
    };
    let Some(name) = item.get("name").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    vec![ToolCallCompleted {
        id: id.to_string(),
        name: name.to_string(),
        arguments: item
            .get("arguments")
            .and_then(|value| value.as_str())
            .unwrap_or("{}")
            .to_string(),
    }]
}

fn stream_error_message(data: &Value, fallback: &str) -> String {
    data.get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| data.get("error"))
        .and_then(|error| {
            error
                .get("message")
                .and_then(|message| message.as_str())
                .or_else(|| error.as_str())
        })
        .or_else(|| data.get("message").and_then(|message| message.as_str()))
        .unwrap_or(fallback)
        .to_string()
}

fn response_input_items(request: &AgentInferenceRequest) -> Vec<Value> {
    let mut items = Vec::new();
    let mut provider_output_call_ids = HashSet::new();

    for conversation_item in &request.conversation {
        let mapped = match conversation_item {
            roder_api::conversation::ConversationItem::UserMessage(message) => Some(json!({
                "type": "message",
                "role": "user",
                "content": user_message_content(message)
            })),
            roder_api::conversation::ConversationItem::AssistantMessage(message) => Some(json!({
                "type": "message",
                "role": "assistant",
                "phase": message.phase.as_deref().unwrap_or(FINAL_ANSWER_PHASE),
                "content": [{ "type": "output_text", "text": message.text }]
            })),
            roder_api::conversation::ConversationItem::ReasoningSummary(summary) => Some(json!({
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": summary.text }]
            })),
            roder_api::conversation::ConversationItem::ToolCall(call) => {
                if provider_output_call_ids.contains(&call.id) {
                    None
                } else {
                    let item_id = fallback_function_call_item_id(&call.id);
                    Some(json!({
                        "type": "function_call",
                        "id": item_id,
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                        "status": "completed"
                    }))
                }
            }
            roder_api::conversation::ConversationItem::ToolResult(result) => Some(json!({
                "type": "function_call_output",
                "call_id": result.id,
                "output": result.result
            })),
            roder_api::conversation::ConversationItem::ContextCompaction(compaction) => {
                Some(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": format!("Context summary:\n{}", compaction.summary) }]
                }))
            }
            roder_api::conversation::ConversationItem::ProviderMetadata(metadata) => {
                append_provider_output_items(metadata, &mut items, &mut provider_output_call_ids);
                None
            }
            _ => None,
        };
        if let Some(item) = mapped {
            items.push(item);
        }
    }

    items
}

fn user_message_content(message: &roder_api::conversation::UserMessage) -> Vec<Value> {
    let mut content = Vec::new();
    if !message.text.is_empty() {
        content.push(json!({ "type": "input_text", "text": message.text }));
    }
    content.extend(message.images.iter().map(|image| {
        json!({
            "type": "input_image",
            "image_url": image.image_url,
        })
    }));
    if content.is_empty() {
        content.push(json!({ "type": "input_text", "text": "" }));
    }
    content
}

fn fallback_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc_") {
        call_id.to_string()
    } else if let Some(suffix) = call_id.strip_prefix("call_") {
        format!("fc_{suffix}")
    } else {
        format!("fc_{call_id}")
    }
}

fn append_provider_output_items(
    metadata: &Value,
    items: &mut Vec<Value>,
    provider_output_call_ids: &mut HashSet<String>,
) {
    let Some(output) = metadata.get("output").and_then(Value::as_array) else {
        return;
    };
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
                    provider_output_call_ids.insert(call_id.to_string());
                }
                items.push(item.clone());
            }
            Some("reasoning") => items.push(item.clone()),
            _ => {}
        }
    }
}

fn extract_response_text(value: &Value) -> String {
    value
        .get("output_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| extract_output_text(value))
        .unwrap_or_default()
}

fn extract_tool_calls(value: &Value) -> Vec<ToolCallCompleted> {
    let Some(output) = value.get("output").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    output
        .iter()
        .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .filter_map(|item| {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            let name = item.get("name").and_then(|v| v.as_str())?.to_string();
            let arguments = item
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}")
                .to_string();
            Some(ToolCallCompleted {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

fn extract_output_text(value: &Value) -> Option<String> {
    let output = value.get("output")?.as_array()?;
    let mut parts = Vec::new();
    for item in output {
        let is_final_answer = item
            .get("phase")
            .and_then(|v| v.as_str())
            .is_none_or(|phase| phase == "final_answer");
        if !is_final_answer {
            continue;
        }
        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if let Some(text) = block
                    .get("text")
                    .or_else(|| block.get("output_text"))
                    .and_then(|v| v.as_str())
                {
                    parts.push(text.to_string());
                }
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(""))
}

fn extract_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt_tokens = number_to_u32(usage.get("input_tokens"));
    let completion_tokens = number_to_u32(usage.get("output_tokens"));
    let total_tokens = number_to_u32(usage.get("total_tokens"))
        .or_else(|| Some(prompt_tokens? + completion_tokens?));
    Some(TokenUsage {
        prompt_tokens: prompt_tokens.unwrap_or_default(),
        completion_tokens: completion_tokens.unwrap_or_default(),
        total_tokens: total_tokens.unwrap_or_default(),
    })
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{
        AssistantMessage, ConversationItem, InputImage, ToolCallRecord, ToolResultRecord,
        UserMessage,
    };
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };

    fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "openai".to_string(),
                model: "gpt-5.5".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("be helpful".to_string()),
                developer: None,
            },
            conversation: vec![
                ConversationItem::UserMessage(UserMessage::text("Hello")),
                ConversationItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
            ],
            tools: vec![roder_api::tools::ToolSpec {
                name: "echo".to_string(),
                description: "echo text".to_string(),
                parameters: json!({ "type": "object" }),
            }],
            tool_choice: roder_api::tools::ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(200),
                temperature: Some(0.2),
                top_p: None,
                response_format: Some(json!({ "type": "json_object" })),
            },
            runtime: RuntimeHints {
                trace_id: None,
                prompt_cache_key: Some("cache-key".to_string()),
            },
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_responses_request_options_and_input_items() {
        let body = OpenAiResponsesEngine::map_request(&request());
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "be helpful");
        assert_eq!(body["max_output_tokens"], 200);
        assert!((body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["prompt_cache_key"], "cache-key");
        assert_eq!(body["text"]["format"]["type"], "json_object");
        assert_eq!(body["tools"][0]["name"], "echo");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][1]["role"], "assistant");
        assert_eq!(body["input"][1]["phase"], "final_answer");
    }

    #[test]
    fn preserves_assistant_message_phase_for_responses_replay() {
        let mut request = request();
        request.conversation = vec![ConversationItem::AssistantMessage(AssistantMessage {
            text: "I will inspect first.".to_string(),
            phase: Some("commentary".to_string()),
        })];

        let input = response_input_items(&request);
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[0]["phase"], "commentary");
        assert_eq!(input[0]["content"][0]["text"], "I will inspect first.");
    }

    #[test]
    fn maps_user_images_to_responses_input_image_content() {
        let mut request = request();
        request.conversation = vec![ConversationItem::UserMessage(UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))];

        let input = response_input_items(&request);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "what is shown?");
        assert_eq!(input[0]["content"][1]["type"], "input_image");
        assert_eq!(
            input[0]["content"][1]["image_url"],
            "data:image/png;base64,YWJj"
        );
    }

    #[test]
    fn maps_apply_patch_tool_for_responses_requests() {
        let mut request = request();
        request.tools = vec![roder_api::tools::ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a patch".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "patch": { "type": "string" } },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }];

        let body = OpenAiResponsesEngine::map_request(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "apply_patch");
        assert_eq!(tools[0]["parameters"]["required"][0], "patch");
    }

    #[test]
    fn replays_provider_function_call_items_before_tool_outputs() {
        let mut request = request();
        request.conversation = vec![
            ConversationItem::UserMessage(UserMessage::text("List files")),
            ConversationItem::ProviderMetadata(json!({
                "output": [
                    {
                        "id": "rs_1",
                        "type": "reasoning",
                        "summary": []
                    },
                    {
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_1",
                        "name": "list_files",
                        "arguments": "{\"path\":\".\"}"
                    }
                ]
            })),
            ConversationItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "list_files".to_string(),
                arguments: "{\"path\":\".\"}".to_string(),
            }),
            ConversationItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("list_files".to_string()),
                result: "Cargo.toml".to_string(),
                is_error: false,
            }),
        ];

        let input = response_input_items(&request);
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[2]["id"], "fc_1");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["status"], "completed");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == "function_call")
                .count(),
            1
        );
    }

    #[test]
    fn fallback_function_call_items_use_responses_item_id_prefix() {
        let mut request = request();
        request.conversation = vec![
            ConversationItem::UserMessage(UserMessage::text("List files")),
            ConversationItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "list_files".to_string(),
                arguments: "{\"path\":\".\"}".to_string(),
            }),
            ConversationItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("list_files".to_string()),
                result: "Cargo.toml".to_string(),
                is_error: false,
            }),
        ];

        let input = response_input_items(&request);
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["id"], "fc_1");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[2]["type"], "function_call_output");
    }

    #[test]
    fn extracts_responses_output_text() {
        let value = json!({
            "output": [{ "content": [{ "text": "hello" }, { "output_text": " world" }] }]
        });
        assert_eq!(extract_output_text(&value).unwrap(), "hello world");
    }

    #[test]
    fn ignores_non_final_responses_output_text() {
        let value = json!({
            "output": [
                { "phase": "analysis", "content": [{ "text": "hidden" }] },
                { "phase": "final_answer", "content": [{ "text": "shown" }] }
            ]
        });
        assert_eq!(extract_response_text(&value), "shown");
    }

    #[test]
    fn extracts_responses_usage() {
        let value = json!({ "usage": { "input_tokens": 3, "output_tokens": 4 } });
        assert_eq!(
            extract_usage(&value),
            Some(TokenUsage {
                prompt_tokens: 3,
                completion_tokens: 4,
                total_tokens: 7,
            })
        );
    }

    #[test]
    fn extracts_responses_tool_calls() {
        let value = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "echo",
                "arguments": "{\"text\":\"hello\"}"
            }]
        });
        let calls = extract_tool_calls(&value);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "echo");
        assert_eq!(calls[0].arguments, "{\"text\":\"hello\"}");
    }

    #[test]
    fn parses_responses_sse_data_frames() {
        let frame = "event: response.output_text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\r\n";
        let event = parse_sse_frame(frame).unwrap().unwrap();
        assert_eq!(event.event.as_deref(), Some("response.output_text.delta"));
        assert_eq!(event.data["delta"], "hi");
        assert_eq!(parse_sse_frame("data: [DONE]\n").unwrap(), None);
    }

    #[test]
    fn emits_streaming_text_and_completed_metadata() {
        let mut state = ResponsesStreamState::default();
        let delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({ "type": "response.output_text.delta", "delta": "hello" }),
        };
        let events = events_from_sse_event(&delta, &mut state);
        assert_eq!(
            events,
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "hello".to_string(),
                phase: None,
            })]
        );

        let completed = SseEvent {
            event: Some("response.completed".to_string()),
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "status": "completed",
                    "output_text": "hello",
                    "usage": {
                        "input_tokens": 3,
                        "output_tokens": 4,
                        "total_tokens": 7
                    }
                }
            }),
        };
        let events = events_from_sse_event(&completed, &mut state);
        assert!(state.terminal);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, InferenceEvent::MessageDelta(_)))
        );
        assert!(events.iter().any(|event| {
            matches!(
                event,
                InferenceEvent::Usage(TokenUsage {
                    prompt_tokens: 3,
                    completion_tokens: 4,
                    total_tokens: 7,
                })
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                InferenceEvent::Completed(CompletionMetadata {
                    provider_response_id: Some(id),
                    ..
                }) if id == "resp_1"
            )
        }));
    }

    #[test]
    fn emits_completed_text_when_no_delta_was_streamed() {
        let mut state = ResponsesStreamState::default();
        let event = SseEvent {
            event: None,
            data: json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "output_text": "fallback"
                }
            }),
        };
        let events = events_from_sse_event(&event, &mut state);
        assert!(matches!(
            events.first(),
            Some(InferenceEvent::MessageDelta(MessageDelta { text, phase })) if text == "fallback" && phase.as_deref() == Some("final_answer")
        ));
    }

    #[test]
    fn emits_phase_text_deltas() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary"
                }
            }),
        };
        assert!(events_from_sse_event(&added, &mut state).is_empty());

        let commentary_delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "delta": "I will inspect first."
            }),
        };
        assert_eq!(
            events_from_sse_event(&commentary_delta, &mut state),
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "I will inspect first.".to_string(),
                phase: Some("commentary".to_string()),
            })]
        );

        let final_added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "msg_2",
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer"
                }
            }),
        };
        assert!(events_from_sse_event(&final_added, &mut state).is_empty());

        let final_delta = SseEvent {
            event: Some("response.output_text.delta".to_string()),
            data: json!({
                "type": "response.output_text.delta",
                "item_id": "msg_2",
                "delta": "Done."
            }),
        };
        assert_eq!(
            events_from_sse_event(&final_delta, &mut state),
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "Done.".to_string(),
                phase: Some("final_answer".to_string()),
            })]
        );
    }

    #[test]
    fn emits_reasoning_text_deltas_and_avoids_done_duplicates() {
        let mut state = ResponsesStreamState::default();
        let delta = SseEvent {
            event: Some("response.reasoning_text.delta".to_string()),
            data: json!({
                "type": "response.reasoning_text.delta",
                "item_id": "rs_1",
                "content_index": 0,
                "delta": "The user is asking "
            }),
        };
        assert_eq!(
            events_from_sse_event(&delta, &mut state),
            vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "The user is asking ".to_string(),
            })]
        );

        let done = SseEvent {
            event: Some("response.reasoning_text.done".to_string()),
            data: json!({
                "type": "response.reasoning_text.done",
                "item_id": "rs_1",
                "content_index": 0,
                "text": "The user is asking for visible thinking."
            }),
        };
        assert!(events_from_sse_event(&done, &mut state).is_empty());
    }

    #[test]
    fn emits_reasoning_text_done_when_no_delta_was_streamed() {
        let mut state = ResponsesStreamState::default();
        let done = SseEvent {
            event: Some("response.reasoning_summary_text.done".to_string()),
            data: json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": "rs_1",
                "content_index": 0,
                "text": "I should inspect the repo."
            }),
        };

        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: "I should inspect the repo.".to_string(),
            })]
        );
    }

    #[test]
    fn emits_tool_call_from_function_arguments_done() {
        let mut state = ResponsesStreamState::default();
        let added = SseEvent {
            event: Some("response.output_item.added".to_string()),
            data: json!({
                "type": "response.output_item.added",
                "item": {
                    "id": "fc_1",
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "echo"
                }
            }),
        };
        assert_eq!(
            events_from_sse_event(&added, &mut state),
            vec![InferenceEvent::ToolCallStarted(ToolCallStarted {
                id: "call_1".to_string(),
                name: "echo".to_string(),
            })]
        );

        let delta = SseEvent {
            event: Some("response.function_call_arguments.delta".to_string()),
            data: json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "delta": "{\"text\":"
            }),
        };
        assert_eq!(
            events_from_sse_event(&delta, &mut state),
            vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                id: "call_1".to_string(),
                arguments_delta: "{\"text\":".to_string(),
            })]
        );

        let done = SseEvent {
            event: Some("response.function_call_arguments.done".to_string()),
            data: json!({
                "type": "response.function_call_arguments.done",
                "item_id": "fc_1",
                "name": "echo",
                "arguments": "{\"text\":\"hello\"}"
            }),
        };
        assert_eq!(
            events_from_sse_event(&done, &mut state),
            vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "echo".to_string(),
                arguments: "{\"text\":\"hello\"}".to_string(),
            })]
        );
    }

    #[test]
    fn emits_stream_failures() {
        let mut state = ResponsesStreamState::default();
        let event = SseEvent {
            event: None,
            data: json!({
                "type": "response.failed",
                "response": {
                    "error": { "message": "bad stream" }
                }
            }),
        };
        let events = events_from_sse_event(&event, &mut state);
        assert!(state.terminal);
        assert_eq!(
            events,
            vec![InferenceEvent::Failed(InferenceFailure {
                message: "bad stream".to_string(),
            })]
        );
    }
}
