use std::collections::BTreeMap;

use roder_api::inference::{
    CompletionMetadata, HostedToolCallCompleted, HostedToolCallStarted, InferenceEvent,
    InferenceFailure, MessageDelta, ReasoningDelta, ToolCallCompleted, ToolCallDelta,
    ToolCallStarted,
};
use roder_api::tools::ToolSpec;
use serde_json::{Value, json};

use crate::provider::{canonical_tool_name, extract_usage, parse_json_object};

/// Caps a single buffered frame so a stream that never sends a frame
/// delimiter cannot grow the buffer without bound.
pub(crate) const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/**
 * Buffers raw bytes so frames split at arbitrary chunk boundaries — including
 * mid multi-byte UTF-8 character — reassemble correctly. Decoding happens only
 * on complete frames; the frame delimiters are ASCII, so a delimiter can never
 * land inside a multi-byte character.
 */
#[derive(Default)]
pub(crate) struct SseFrameBuffer {
    bytes: Vec<u8>,
    /**
     * Length of the prefix already scanned without finding a delimiter; the
     * next scan resumes just before it instead of rescanning from the start.
     */
    scanned: usize,
}

impl SseFrameBuffer {
    pub(crate) fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    /// False once the unterminated frame exceeds `MAX_FRAME_BYTES`.
    pub(crate) fn within_frame_cap(&self) -> bool {
        self.bytes.len() <= MAX_FRAME_BYTES
    }

    pub(crate) fn take_frame(&mut self) -> Option<String> {
        // A delimiter (at most 4 bytes) may span the scanned boundary.
        let start = self.scanned.saturating_sub(3);
        let Some((idx, delimiter_len)) = frame_boundary(&self.bytes, start) else {
            self.scanned = self.bytes.len();
            return None;
        };
        let frame = String::from_utf8_lossy(&self.bytes[..idx]).into_owned();
        self.bytes.drain(..idx + delimiter_len);
        self.scanned = 0;
        Some(frame)
    }

    pub(crate) fn take_trailing(&mut self) -> Option<String> {
        self.scanned = 0;
        if self.bytes.iter().all(u8::is_ascii_whitespace) {
            self.bytes.clear();
            return None;
        }
        let frame = String::from_utf8_lossy(&self.bytes).into_owned();
        self.bytes.clear();
        Some(frame)
    }
}

fn frame_boundary(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let haystack = bytes.get(start..)?;
    let lf = find_subsequence(haystack, b"\n\n").map(|idx| (start + idx, 2));
    let crlf = find_subsequence(haystack, b"\r\n\r\n").map(|idx| (start + idx, 4));
    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (lf, crlf) => lf.or(crlf),
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

enum ContentBlock {
    Text {
        text: String,
        citations: Vec<Value>,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        wire_name: String,
        partial_json: String,
    },
    /**
     * Anthropic `server_tool_use` tool-search blocks map to the canonical
     * hosted tool-call lifecycle so provider-native tool search stays visible
     * in the same timeline as other hosted tools, while the block value is
     * still accumulated for ProviderMetadata reconstruction.
     */
    ToolSearchUse {
        id: String,
        value: Value,
        partial_json: String,
    },
    /**
     * Remaining provider-native blocks (e.g. other server_tool_use kinds and
     * tool_search_tool_result) emit no inference events, but their streamed
     * input is still accumulated so the reconstructed ProviderMetadata
     * matches the non-streaming response.
     */
    Other {
        value: Value,
        partial_json: String,
    },
}

/**
 * Incrementally maps Anthropic Messages SSE frames to inference events while
 * reconstructing the final message value (content, stop_reason, usage) so the
 * terminal ProviderMetadata event matches the non-streaming response shape.
 */
pub(crate) struct AnthropicStreamState {
    tools: Vec<ToolSpec>,
    blocks: BTreeMap<u64, ContentBlock>,
    message: Value,
    usage: Option<Value>,
    stop_reason: Option<String>,
    pub(crate) terminal: bool,
}

impl AnthropicStreamState {
    pub(crate) fn new(tools: Vec<ToolSpec>) -> Self {
        Self {
            tools,
            blocks: BTreeMap::new(),
            message: json!({}),
            usage: None,
            stop_reason: None,
            terminal: false,
        }
    }

    pub(crate) fn push_frame(&mut self, frame: &str) -> anyhow::Result<Vec<InferenceEvent>> {
        let mut event_name = None;
        let mut data_lines = Vec::new();
        for raw_line in frame.lines() {
            let line = raw_line.trim_end_matches('\r');
            if let Some(value) = line.strip_prefix("event:") {
                event_name = Some(value.trim_start().to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim_start());
            }
        }
        if data_lines.is_empty() {
            return Ok(Vec::new());
        }
        let data: Value = serde_json::from_str(&data_lines.join("\n"))
            .map_err(|err| anyhow::anyhow!("failed to parse Anthropic SSE data as JSON: {err}"))?;
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(event_name)
            .unwrap_or_default();
        Ok(self.apply(&kind, &data))
    }

    fn apply(&mut self, kind: &str, data: &Value) -> Vec<InferenceEvent> {
        match kind {
            "message_start" => {
                if let Some(message) = data.get("message").filter(|value| value.is_object()) {
                    self.message = message.clone();
                }
                if let Some(usage) = self.message.get("usage") {
                    merge_usage(&mut self.usage, usage);
                }
                self.message["content"] = json!([]);
                Vec::new()
            }
            "content_block_start" => self.start_block(data),
            "content_block_delta" => self.apply_delta(data),
            "content_block_stop" => self.stop_block(data),
            "message_delta" => {
                if let Some(delta) = data.get("delta").and_then(Value::as_object) {
                    for (key, value) in delta {
                        self.message[key] = value.clone();
                    }
                    if let Some(stop_reason) = delta.get("stop_reason").and_then(Value::as_str) {
                        self.stop_reason = Some(stop_reason.to_string());
                    }
                }
                if let Some(usage) = data.get("usage") {
                    merge_usage(&mut self.usage, usage);
                }
                Vec::new()
            }
            "message_stop" => self.finish(),
            // Provider-signaled errors (e.g. overloaded_error on an HTTP 200
            // connection) end the turn as a terminal Failed event, mirroring
            // the openai-responses provider; the stream itself stays Ok so
            // the host records exactly one turn failure.
            "error" => {
                let error_type = data
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or("api_error");
                let message = data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                self.terminal = true;
                vec![InferenceEvent::Failed(InferenceFailure {
                    message: format!("Anthropic stream error ({error_type}): {message}"),
                })]
            }
            // ping and unknown event types are skipped.
            _ => Vec::new(),
        }
    }

    fn start_block(&mut self, data: &Value) -> Vec<InferenceEvent> {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let block = data
            .get("content_block")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match block_type {
            "text" => {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let events = if text.is_empty() {
                    Vec::new()
                } else {
                    vec![InferenceEvent::MessageDelta(MessageDelta {
                        text: text.clone(),
                        phase: None,
                    })]
                };
                self.blocks.insert(
                    index,
                    ContentBlock::Text {
                        text,
                        citations: Vec::new(),
                    },
                );
                events
            }
            "thinking" => {
                let thinking = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let events = if thinking.is_empty() {
                    Vec::new()
                } else {
                    vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: thinking.clone(),
                    })]
                };
                self.blocks.insert(
                    index,
                    ContentBlock::Thinking {
                        thinking,
                        signature: String::new(),
                    },
                );
                events
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let wire_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = canonical_tool_name(&wire_name, &self.tools);
                self.blocks.insert(
                    index,
                    ContentBlock::ToolUse {
                        id: id.clone(),
                        wire_name,
                        partial_json: String::new(),
                    },
                );
                vec![InferenceEvent::ToolCallStarted(ToolCallStarted {
                    id,
                    name,
                })]
            }
            "server_tool_use"
                if block.get("name").and_then(Value::as_str) == Some("tool_search") =>
            {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.blocks.insert(
                    index,
                    ContentBlock::ToolSearchUse {
                        id: id.clone(),
                        value: block,
                        partial_json: String::new(),
                    },
                );
                vec![InferenceEvent::HostedToolCallStarted(HostedToolCallStarted {
                    id,
                    name: "tool_search".to_string(),
                })]
            }
            _ => {
                self.blocks.insert(
                    index,
                    ContentBlock::Other {
                        value: block,
                        partial_json: String::new(),
                    },
                );
                Vec::new()
            }
        }
    }

    fn apply_delta(&mut self, data: &Value) -> Vec<InferenceEvent> {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(delta) = data.get("delta") else {
            return Vec::new();
        };
        let delta_type = delta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(block) = self.blocks.get_mut(&index) else {
            return Vec::new();
        };
        match (delta_type, block) {
            ("text_delta", ContentBlock::Text { text, .. }) => {
                let Some(chunk) = delta.get("text").and_then(Value::as_str) else {
                    return Vec::new();
                };
                if chunk.is_empty() {
                    return Vec::new();
                }
                text.push_str(chunk);
                vec![InferenceEvent::MessageDelta(MessageDelta {
                    text: chunk.to_string(),
                    phase: None,
                })]
            }
            ("thinking_delta", ContentBlock::Thinking { thinking, .. }) => {
                let Some(chunk) = delta.get("thinking").and_then(Value::as_str) else {
                    return Vec::new();
                };
                if chunk.is_empty() {
                    return Vec::new();
                }
                thinking.push_str(chunk);
                vec![InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: chunk.to_string(),
                })]
            }
            ("signature_delta", ContentBlock::Thinking { signature, .. }) => {
                if let Some(chunk) = delta.get("signature").and_then(Value::as_str) {
                    signature.push_str(chunk);
                }
                Vec::new()
            }
            (
                "input_json_delta",
                ContentBlock::ToolUse {
                    id, partial_json, ..
                },
            ) => {
                let Some(chunk) = delta.get("partial_json").and_then(Value::as_str) else {
                    return Vec::new();
                };
                if chunk.is_empty() {
                    return Vec::new();
                }
                partial_json.push_str(chunk);
                vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id: id.clone(),
                    arguments_delta: chunk.to_string(),
                })]
            }
            (
                "input_json_delta",
                ContentBlock::ToolSearchUse { partial_json, .. }
                | ContentBlock::Other { partial_json, .. },
            ) => {
                if let Some(chunk) = delta.get("partial_json").and_then(Value::as_str) {
                    partial_json.push_str(chunk);
                }
                Vec::new()
            }
            ("citations_delta", ContentBlock::Text { citations, .. }) => {
                if let Some(citation) = delta.get("citation") {
                    citations.push(citation.clone());
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn stop_block(&mut self, data: &Value) -> Vec<InferenceEvent> {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        self.close_block(index)
    }

    fn close_block(&mut self, index: u64) -> Vec<InferenceEvent> {
        let Some(block) = self.blocks.remove(&index) else {
            return Vec::new();
        };
        let (value, events) = match block {
            ContentBlock::Text { text, citations } => {
                let mut value = json!({ "type": "text", "text": text });
                if !citations.is_empty() {
                    value["citations"] = json!(citations);
                }
                (value, Vec::new())
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => (
                json!({ "type": "thinking", "thinking": thinking, "signature": signature }),
                Vec::new(),
            ),
            ContentBlock::ToolUse {
                id,
                wire_name,
                partial_json,
            } => {
                let input = parse_json_object(&partial_json);
                let call = ToolCallCompleted {
                    id: id.clone(),
                    name: canonical_tool_name(&wire_name, &self.tools),
                    arguments: input.to_string(),
                };
                (
                    json!({ "type": "tool_use", "id": id, "name": wire_name, "input": input }),
                    vec![InferenceEvent::ToolCallCompleted(call)],
                )
            }
            ContentBlock::ToolSearchUse {
                id,
                mut value,
                partial_json,
            } => {
                let input = if partial_json.is_empty() {
                    value.get("input").cloned().unwrap_or_else(|| json!({}))
                } else {
                    parse_json_object(&partial_json)
                };
                value["input"] = input.clone();
                let call = HostedToolCallCompleted {
                    id,
                    name: "tool_search".to_string(),
                    arguments: input.to_string(),
                };
                (
                    value,
                    vec![InferenceEvent::HostedToolCallCompleted(call)],
                )
            }
            ContentBlock::Other {
                mut value,
                partial_json,
            } => {
                if !partial_json.is_empty() {
                    value["input"] = parse_json_object(&partial_json);
                }
                (value, Vec::new())
            }
        };
        match self
            .message
            .get_mut("content")
            .and_then(Value::as_array_mut)
        {
            Some(content) => content.push(value),
            None => self.message["content"] = json!([value]),
        }
        events
    }

    fn finish(&mut self) -> Vec<InferenceEvent> {
        if self.terminal {
            return Vec::new();
        }
        self.terminal = true;
        let mut events = Vec::new();
        // message_stop with still-open blocks is a protocol violation; flush
        // them (in index order) so accumulated text is not silently dropped
        // and started tool calls still complete.
        let open_blocks: Vec<u64> = self.blocks.keys().copied().collect();
        for index in open_blocks {
            events.extend(self.close_block(index));
        }
        if let Some(usage_value) = &self.usage {
            self.message["usage"] = usage_value.clone();
            if let Some(usage) = extract_usage(&self.message) {
                events.push(InferenceEvent::Usage(usage));
            }
        }
        events.push(InferenceEvent::ProviderMetadata(self.message.clone()));
        events.push(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: self.stop_reason.clone(),
            provider_response_id: self
                .message
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
        }));
        events
    }
}

/// Skips null fields so a partial `message_delta` usage (e.g. only
/// `output_tokens`) never clears input/cache counts captured at
/// `message_start`.
fn merge_usage(target: &mut Option<Value>, incoming: &Value) {
    let Some(incoming) = incoming.as_object() else {
        return;
    };
    let target = target.get_or_insert_with(|| json!({}));
    for (key, value) in incoming {
        if !value.is_null() {
            target[key.as_str()] = value.clone();
        }
    }
}

#[cfg(test)]
#[path = "sse_tool_search_tests.rs"]
mod sse_tool_search_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::TokenUsage;

    fn frames(state: &mut AnthropicStreamState, frames: &[&str]) -> Vec<InferenceEvent> {
        frames
            .iter()
            .flat_map(|frame| state.push_frame(frame).unwrap())
            .collect()
    }

    #[test]
    fn frame_buffer_splits_multiple_frames_per_chunk() {
        let mut buffer = SseFrameBuffer::default();
        buffer.push(b"event: ping\ndata: {\"type\":\"ping\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
        assert_eq!(
            buffer.take_frame(),
            Some("event: ping\ndata: {\"type\":\"ping\"}".to_string())
        );
        assert_eq!(
            buffer.take_frame(),
            Some("event: message_stop\ndata: {\"type\":\"message_stop\"}".to_string())
        );
        assert_eq!(buffer.take_frame(), None);
        assert_eq!(buffer.take_trailing(), None);
    }

    #[test]
    fn frame_buffer_reassembles_frames_split_across_chunks() {
        let mut buffer = SseFrameBuffer::default();
        buffer.push(b"data: {\"type\":");
        assert_eq!(buffer.take_frame(), None);
        buffer.push(b"\"ping\"}\n\ndata: tail");
        assert_eq!(
            buffer.take_frame(),
            Some("data: {\"type\":\"ping\"}".to_string())
        );
        assert_eq!(buffer.take_frame(), None);
        assert_eq!(buffer.take_trailing(), Some("data: tail".to_string()));
    }

    #[test]
    fn frame_buffer_finds_delimiter_spanning_scanned_boundary() {
        let mut buffer = SseFrameBuffer::default();
        buffer.push(b"data: x\r\n");
        assert_eq!(buffer.take_frame(), None);
        buffer.push(b"\r\ndata: y\n");
        assert_eq!(buffer.take_frame(), Some("data: x".to_string()));
        assert_eq!(buffer.take_frame(), None);
        buffer.push(b"\n");
        assert_eq!(buffer.take_frame(), Some("data: y".to_string()));
    }

    #[test]
    fn frame_buffer_reports_frame_cap_overflow() {
        let mut buffer = SseFrameBuffer::default();
        buffer.push(&vec![b'a'; MAX_FRAME_BYTES + 1]);
        assert_eq!(buffer.take_frame(), None);
        assert!(!buffer.within_frame_cap());
    }

    #[test]
    fn frame_buffer_handles_crlf_delimiters() {
        let mut buffer = SseFrameBuffer::default();
        buffer.push(b"event: ping\r\ndata: {\"type\":\"ping\"}\r\n\r\ndata: x\n\n");
        assert_eq!(
            buffer.take_frame(),
            Some("event: ping\r\ndata: {\"type\":\"ping\"}".to_string())
        );
        assert_eq!(buffer.take_frame(), Some("data: x".to_string()));
    }

    #[test]
    fn frame_buffer_reassembles_multibyte_utf8_split_across_chunks() {
        let frame = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"🦀\"}}\n\n";
        let bytes = frame.as_bytes();
        // Split inside the 4-byte crab scalar.
        let split = frame.find("🦀").unwrap() + 2;
        let mut buffer = SseFrameBuffer::default();
        buffer.push(&bytes[..split]);
        assert_eq!(buffer.take_frame(), None);
        buffer.push(&bytes[split..]);
        let reassembled = buffer.take_frame().unwrap();
        assert!(reassembled.contains("🦀"), "{reassembled}");
        assert!(!reassembled.contains('\u{FFFD}'), "{reassembled}");
    }

    #[test]
    fn maps_text_tool_use_and_usage_frames_to_events() {
        let mut state = AnthropicStreamState::new(vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }]);
        let events = frames(
            &mut state,
            &[
                r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":1,"cache_read_input_tokens":8,"output_tokens":1}}}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"data: {"type":"ping"}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_2","name":"shell","input":{}}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":"}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":1}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":7}}"#,
                r#"data: {"type":"message_stop"}"#,
            ],
        );

        assert_eq!(
            events,
            vec![
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "hello".to_string(),
                    phase: None,
                }),
                InferenceEvent::MessageDelta(MessageDelta {
                    text: " world".to_string(),
                    phase: None,
                }),
                InferenceEvent::ToolCallStarted(ToolCallStarted {
                    id: "toolu_2".to_string(),
                    name: "shell".to_string(),
                }),
                InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id: "toolu_2".to_string(),
                    arguments_delta: r#"{"cmd":"#.to_string(),
                }),
                InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id: "toolu_2".to_string(),
                    arguments_delta: r#""ls"}"#.to_string(),
                }),
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "toolu_2".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"ls"}"#.to_string(),
                }),
                InferenceEvent::Usage(
                    TokenUsage::new(11, 7, 18)
                        .with_cached_prompt_tokens(8)
                        .with_cache_creation_prompt_tokens(1)
                ),
                InferenceEvent::ProviderMetadata(json!({
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-sonnet-4-6",
                    "stop_reason": "tool_use",
                    "stop_sequence": null,
                    "content": [
                        { "type": "text", "text": "hello world" },
                        { "type": "tool_use", "id": "toolu_2", "name": "shell", "input": { "cmd": "ls" } }
                    ],
                    "usage": {
                        "input_tokens": 2,
                        "cache_creation_input_tokens": 1,
                        "cache_read_input_tokens": 8,
                        "output_tokens": 7
                    }
                })),
                InferenceEvent::Completed(CompletionMetadata {
                    stop_reason: Some("tool_use".to_string()),
                    provider_response_id: Some("msg_1".to_string()),
                }),
            ]
        );
        assert!(state.terminal);
    }

    #[test]
    fn maps_thinking_deltas_to_reasoning_events() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = frames(
            &mut state,
            &[
                r#"data: {"type":"message_start","message":{"id":"msg_1","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"let me"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" think"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
        );

        assert_eq!(
            events,
            vec![
                InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: "let me".to_string(),
                }),
                InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: " think".to_string(),
                }),
            ]
        );
        assert_eq!(
            state.message["content"][0],
            json!({ "type": "thinking", "thinking": "let me think", "signature": "sig" })
        );
    }

    #[test]
    fn tool_use_without_input_deltas_completes_with_empty_arguments() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = frames(
            &mut state,
            &[
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"noop"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
        );

        assert_eq!(
            events,
            vec![
                InferenceEvent::ToolCallStarted(ToolCallStarted {
                    id: "toolu_1".to_string(),
                    name: "noop".to_string(),
                }),
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "toolu_1".to_string(),
                    name: "noop".to_string(),
                    arguments: "{}".to_string(),
                }),
            ]
        );
    }

    #[test]
    fn maps_sanitized_wire_tool_names_back_to_canonical_names() {
        let tools = vec![ToolSpec {
            name: "webwright.run_script".to_string(),
            description: "Run a webwright script".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        }];
        let mut state = AnthropicStreamState::new(tools);
        let events = frames(
            &mut state,
            &[
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"webwright__run_script"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"script\":\"x\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
        );

        assert_eq!(
            events,
            vec![
                InferenceEvent::ToolCallStarted(ToolCallStarted {
                    id: "toolu_1".to_string(),
                    name: "webwright.run_script".to_string(),
                }),
                InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id: "toolu_1".to_string(),
                    arguments_delta: r#"{"script":"x"}"#.to_string(),
                }),
                InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "toolu_1".to_string(),
                    name: "webwright.run_script".to_string(),
                    arguments: r#"{"script":"x"}"#.to_string(),
                }),
            ]
        );
        // The reconstructed metadata keeps the wire name, matching the raw
        // non-streaming response shape.
        assert_eq!(state.message["content"][0]["name"], "webwright__run_script");
    }

    #[test]
    fn skips_unknown_event_types_and_blocks() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = frames(
            &mut state,
            &[
                r#"event: fancy_new_event
data: {"type":"fancy_new_event","payload":1}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srv_1"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"citation_delta","citation":{}}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
            ],
        );

        assert_eq!(events, Vec::new());
        assert_eq!(
            state.message["content"][0],
            json!({ "type": "server_tool_use", "id": "srv_1" })
        );
    }

    #[test]
    fn error_frame_maps_to_terminal_failed_event() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = state
            .push_frame(
                r#"event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            )
            .unwrap();

        assert_eq!(
            events,
            vec![InferenceEvent::Failed(InferenceFailure {
                message: "Anthropic stream error (overloaded_error): Overloaded".to_string(),
            })]
        );
        assert!(state.terminal);
    }

    #[test]
    fn message_stop_flushes_still_open_blocks() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = frames(
            &mut state,
            &[
                r#"data: {"type":"message_start","message":{"id":"msg_1","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"noop"}}"#,
                // message_stop without content_block_stop for either block.
                r#"data: {"type":"message_stop"}"#,
            ],
        );

        assert!(
            events.contains(&InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "toolu_1".to_string(),
                name: "noop".to_string(),
                arguments: "{}".to_string(),
            }))
        );
        assert_eq!(
            state.message["content"],
            json!([
                { "type": "text", "text": "hello" },
                { "type": "tool_use", "id": "toolu_1", "name": "noop", "input": {} }
            ])
        );
        assert!(matches!(events.last(), Some(InferenceEvent::Completed(_))));
    }

    #[test]
    fn accumulates_server_tool_use_input_and_text_citations() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let events = frames(
            &mut state,
            &[
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srv_1","name":"tool_search","input":{}}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"shell\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
                r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"cited"}}"#,
                r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location","url":"https://example.com"}}}"#,
                r#"data: {"type":"content_block_stop","index":1}"#,
            ],
        );

        // tool_search server_tool_use blocks map to canonical hosted tool
        // events; the accumulated input becomes the completion arguments.
        assert_eq!(
            events,
            vec![
                InferenceEvent::HostedToolCallStarted(HostedToolCallStarted {
                    id: "srv_1".to_string(),
                    name: "tool_search".to_string(),
                }),
                InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted {
                    id: "srv_1".to_string(),
                    name: "tool_search".to_string(),
                    arguments: r#"{"query":"shell"}"#.to_string(),
                }),
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "cited".to_string(),
                    phase: None,
                })
            ]
        );
        assert_eq!(
            state.message["content"],
            json!([
                {
                    "type": "server_tool_use",
                    "id": "srv_1",
                    "name": "tool_search",
                    "input": { "query": "shell" }
                },
                {
                    "type": "text",
                    "text": "cited",
                    "citations": [
                        { "type": "web_search_result_location", "url": "https://example.com" }
                    ]
                }
            ])
        );
    }

    #[test]
    fn invalid_json_data_surfaces_as_stream_error() {
        let mut state = AnthropicStreamState::new(Vec::new());
        let error = state.push_frame("data: {not json").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to parse Anthropic SSE data as JSON"),
            "{error}"
        );
    }

    #[test]
    fn frames_without_data_are_skipped() {
        let mut state = AnthropicStreamState::new(Vec::new());
        assert_eq!(
            state.push_frame(": keep-alive comment").unwrap(),
            Vec::new()
        );
        assert_eq!(state.push_frame("event: ping").unwrap(), Vec::new());
    }
}
