use std::collections::{BTreeMap, HashMap, HashSet};

use futures::StreamExt;
use roder_api::conversation::ConversationItem;
use roder_api::inference::{
    AgentInferenceRequest, CompletionMetadata, InferenceEvent, InferenceEventStream, MessageDelta,
    ReasoningDelta, TokenUsage, ToolCallCompleted,
};
use roder_api::tools::ToolChoice;
use serde_json::{Value, json};

pub async fn stream_chat_completions(
    base_url: &str,
    api_key: &str,
    request: AgentInferenceRequest,
) -> anyhow::Result<InferenceEventStream> {
    let (tools, tool_name_map) = chat_tools(&request);
    let estimated_prompt_tokens = estimate_prompt_tokens(&request, &tools);
    let mut body = json!({
        "model": request.model.model,
        "messages": chat_messages(&request, &tool_name_map),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    body["chat_template_kwargs"] = json!({
        "enable_thinking": poolside_thinking_enabled(&request),
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = chat_tool_choice(&request.tool_choice, &tool_name_map);
        body["parallel_tool_calls"] = json!(request.runtime.parallel_tool_calls.unwrap_or(true));
    }
    if let Some(max_tokens) = request.output.max_tokens {
        body["max_tokens"] = json!(max_tokens);
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

    let response = reqwest::Client::new()
        .post(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Poolside Chat Completions error {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
    }

    let mut bytes = response.bytes_stream();
    let stream = async_stream::try_stream! {
        let mut state = ChatStreamState::new(tool_name_map, estimated_prompt_tokens);
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

#[derive(Debug, Default)]
struct ChatToolNameMap {
    tool_name_to_api_name: HashMap<String, String>,
    api_name_to_tool_name: HashMap<String, String>,
}

impl ChatToolNameMap {
    fn register(&mut self, tool_name: &str, api_name: &str) {
        self.tool_name_to_api_name
            .insert(tool_name.to_string(), api_name.to_string());
        self.api_name_to_tool_name
            .insert(api_name.to_string(), tool_name.to_string());
    }

    fn api_name<'a>(&'a self, tool_name: &'a str) -> &'a str {
        self.tool_name_to_api_name
            .get(tool_name)
            .map_or(tool_name, String::as_str)
    }

    fn canonical_name<'a>(&'a self, api_name: &'a str) -> &'a str {
        self.api_name_to_tool_name
            .get(api_name)
            .map_or(api_name, String::as_str)
    }
}

fn chat_tools(request: &AgentInferenceRequest) -> (Vec<Value>, ChatToolNameMap) {
    let mut tools = Vec::new();
    let mut used_tool_names = HashSet::new();
    let mut tool_name_map = ChatToolNameMap::default();
    for tool in &request.tools {
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
    for item in &request.conversation {
        match item {
            ConversationItem::UserMessage(message) => {
                messages.push(json!({ "role": "user", "content": message.text }));
            }
            ConversationItem::AssistantMessage(message) => {
                if !message.text.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": message.text }));
                }
            }
            ConversationItem::ToolCall(call) => {
                messages.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": tool_name_map.api_name(&call.name),
                            "arguments": call.arguments,
                        },
                    }],
                }));
            }
            ConversationItem::ToolResult(result) => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": result.id,
                    "content": result.result,
                }));
            }
            ConversationItem::ContextCompaction(compaction) => {
                messages.push(json!({ "role": "system", "content": compaction.summary }));
            }
            ConversationItem::ReasoningSummary(summary) => {
                messages.push(json!({ "role": "assistant", "content": summary.text }));
            }
            ConversationItem::FileChange(_)
            | ConversationItem::Error(_)
            | ConversationItem::ProviderMetadata(_) => {}
        }
    }
    messages
}

#[derive(Debug)]
struct ChatStreamState {
    buffer: String,
    tool_name_map: ChatToolNameMap,
    tool_calls: BTreeMap<u64, PartialChatToolCall>,
    stop_reason: Option<String>,
    provider_response_id: Option<String>,
    usage: Option<TokenUsage>,
    metadata_chunks: Vec<Value>,
    estimated_prompt_tokens: u32,
    estimated_completion_tokens: u32,
}

#[derive(Debug, Default)]
struct PartialChatToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ChatStreamState {
    fn new(tool_name_map: ChatToolNameMap, estimated_prompt_tokens: u32) -> Self {
        Self {
            buffer: String::new(),
            tool_name_map,
            tool_calls: BTreeMap::new(),
            stop_reason: None,
            provider_response_id: None,
            usage: None,
            metadata_chunks: Vec::new(),
            estimated_prompt_tokens,
            estimated_completion_tokens: 0,
        }
    }

    fn push_chunk(&mut self, chunk: &[u8]) -> anyhow::Result<Vec<InferenceEvent>> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut events = Vec::new();
        while let Some((pos, separator_len)) = sse_frame_boundary(&self.buffer) {
            let frame = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + separator_len);
            events.extend(self.push_sse_frame(&frame)?);
        }
        Ok(events)
    }

    fn push_sse_frame(&mut self, frame: &str) -> anyhow::Result<Vec<InferenceEvent>> {
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return Ok(Vec::new());
        }

        let value: Value = serde_json::from_str(&data)?;
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            self.provider_response_id = Some(id.to_string());
        }
        if let Some(usage) = extract_chat_usage(&value) {
            self.usage = Some(usage);
        }
        self.metadata_chunks.push(value.clone());

        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(Vec::new());
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str)
            && !reason.is_empty()
        {
            self.stop_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(Vec::new());
        };

        let mut events = Vec::new();
        if let Some(reasoning) = first_string(delta, &["reasoning_content", "reasoning"])
            && !reasoning.trim().is_empty()
        {
            self.estimated_completion_tokens = self
                .estimated_completion_tokens
                .saturating_add(estimate_text_tokens(reasoning));
            events.push(InferenceEvent::ReasoningDelta(ReasoningDelta {
                text: reasoning.to_string(),
            }));
        }
        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.trim().is_empty()
        {
            self.estimated_completion_tokens = self
                .estimated_completion_tokens
                .saturating_add(estimate_text_tokens(content));
            events.push(InferenceEvent::MessageDelta(MessageDelta {
                text: content.to_string(),
                phase: None,
            }));
        }
        for tool_call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let partial = self.tool_calls.entry(index).or_default();
            if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                partial.id = Some(id.to_string());
            }
            if let Some(function) = tool_call.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    partial.name = Some(name.to_string());
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    self.estimated_completion_tokens = self
                        .estimated_completion_tokens
                        .saturating_add(estimate_text_tokens(arguments));
                    partial.arguments.push_str(arguments);
                }
            }
        }
        Ok(events)
    }

    fn finish(self) -> Vec<InferenceEvent> {
        let mut events = Vec::new();
        for (_, call) in self.tool_calls {
            let Some(id) = call.id else {
                continue;
            };
            let Some(name) = call.name else {
                continue;
            };
            events.push(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id,
                name: self.tool_name_map.canonical_name(&name).to_string(),
                arguments: call.arguments,
            }));
        }
        events.push(InferenceEvent::Usage(self.usage.unwrap_or_else(|| {
            let completion_tokens = self.estimated_completion_tokens.max(1);
            TokenUsage {
                prompt_tokens: self.estimated_prompt_tokens,
                completion_tokens,
                total_tokens: self
                    .estimated_prompt_tokens
                    .saturating_add(completion_tokens),
            }
        })));
        events.push(InferenceEvent::ProviderMetadata(json!({
            "chunks": self.metadata_chunks,
        })));
        events.push(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: self.stop_reason,
            provider_response_id: self.provider_response_id,
        }));
        events
    }
}

fn extract_chat_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt_tokens = first_u32(
        usage,
        &[
            "prompt_tokens",
            "input_tokens",
            "promptTokens",
            "inputTokens",
        ],
    );
    let completion_tokens = first_u32(
        usage,
        &[
            "completion_tokens",
            "output_tokens",
            "completionTokens",
            "outputTokens",
        ],
    );
    let total_tokens = first_u32(usage, &["total_tokens", "totalTokens"]);
    if prompt_tokens.is_none() && completion_tokens.is_none() && total_tokens.is_none() {
        return None;
    }
    let prompt_tokens = prompt_tokens.unwrap_or_default();
    let completion_tokens = completion_tokens.unwrap_or_default();
    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: total_tokens
            .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens)),
    })
}

fn first_u32(value: &Value, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| number_to_u32(value.get(*key)))
}

fn first_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn poolside_thinking_enabled(request: &AgentInferenceRequest) -> bool {
    if !request.reasoning.enabled {
        return false;
    }
    request.reasoning.level.as_deref() != Some("none")
}

fn estimate_prompt_tokens(request: &AgentInferenceRequest, tools: &[Value]) -> u32 {
    let mut chars = request
        .instructions
        .system
        .as_deref()
        .unwrap_or_default()
        .len()
        + request
            .instructions
            .developer
            .as_deref()
            .unwrap_or_default()
            .len();
    for item in &request.conversation {
        chars = chars.saturating_add(match item {
            ConversationItem::UserMessage(message) => message.text.len(),
            ConversationItem::AssistantMessage(message) => message.text.len(),
            ConversationItem::ToolCall(call) => {
                call.name.len().saturating_add(call.arguments.len())
            }
            ConversationItem::ToolResult(result) => result.result.len(),
            ConversationItem::ContextCompaction(compaction) => compaction.summary.len(),
            ConversationItem::ReasoningSummary(summary) => summary.text.len(),
            ConversationItem::FileChange(_)
            | ConversationItem::Error(_)
            | ConversationItem::ProviderMetadata(_) => 0,
        });
    }
    chars = chars.saturating_add(
        tools
            .iter()
            .map(|tool| tool.to_string().len())
            .sum::<usize>(),
    );
    estimate_tokens_from_chars(chars).max(1)
}

fn estimate_text_tokens(text: &str) -> u32 {
    estimate_tokens_from_chars(text.len())
}

fn estimate_tokens_from_chars(chars: usize) -> u32 {
    u32::try_from(chars.saturating_add(3) / 4).unwrap_or(u32::MAX)
}

fn sse_frame_boundary(buffer: &str) -> Option<(usize, usize)> {
    match (buffer.find("\n\n"), buffer.find("\r\n\r\n")) {
        (Some(lf), Some(crlf)) if lf < crlf => Some((lf, 2)),
        (Some(_), Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
    }
}
