use std::collections::{BTreeMap, HashMap};

use roder_api::inference::{
    CompletionMetadata, InferenceEvent, MessageDelta, TokenUsage, ToolCallCompleted,
};
use serde_json::{Value, json};

#[derive(Debug, Default)]
pub(crate) struct ChatToolNameMap {
    tool_name_to_api_name: HashMap<String, String>,
    api_name_to_tool_name: HashMap<String, String>,
}

impl ChatToolNameMap {
    pub(crate) fn register(&mut self, tool_name: &str, api_name: &str) {
        self.tool_name_to_api_name
            .insert(tool_name.to_string(), api_name.to_string());
        self.api_name_to_tool_name
            .insert(api_name.to_string(), tool_name.to_string());
    }

    pub(crate) fn api_name<'a>(&'a self, tool_name: &'a str) -> &'a str {
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

#[derive(Debug)]
pub(crate) struct ChatStreamState {
    buffer: String,
    tool_name_map: ChatToolNameMap,
    tool_calls: BTreeMap<u64, PartialChatToolCall>,
    stop_reason: Option<String>,
    provider_response_id: Option<String>,
    usage: Option<TokenUsage>,
    metadata_chunks: Vec<Value>,
}

impl ChatStreamState {
    pub(crate) fn new(tool_name_map: ChatToolNameMap) -> Self {
        Self {
            buffer: String::new(),
            tool_name_map,
            tool_calls: BTreeMap::new(),
            stop_reason: None,
            provider_response_id: None,
            usage: None,
            metadata_chunks: Vec::new(),
        }
    }

    pub(crate) fn push_chunk(&mut self, chunk: &[u8]) -> anyhow::Result<Vec<InferenceEvent>> {
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
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(Vec::new());
        };

        let mut events = Vec::new();
        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
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
                    partial.arguments.push_str(arguments);
                }
            }
        }
        Ok(events)
    }

    pub(crate) fn finish(self) -> Vec<InferenceEvent> {
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
        if let Some(usage) = self.usage {
            events.push(InferenceEvent::Usage(usage));
        }
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

#[derive(Debug, Default)]
struct PartialChatToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn extract_chat_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let cached_prompt_tokens = number_to_u32(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| number_to_u32(usage.pointer("/input_tokens_details/cached_tokens")))
        .unwrap_or_default();
    Some(
        TokenUsage::new(
            number_to_u32(usage.get("prompt_tokens")).unwrap_or_default(),
            number_to_u32(usage.get("completion_tokens")).unwrap_or_default(),
            number_to_u32(usage.get("total_tokens")).unwrap_or_default(),
        )
        .with_cached_prompt_tokens(cached_prompt_tokens),
    )
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
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
