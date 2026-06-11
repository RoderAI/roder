use roder_api::inference::{
    CompletionMetadata, InferenceEvent, InferenceFailure, MessageDelta, ReasoningDelta,
};
use serde_json::{Value, json};

use crate::mapping::{
    canonical_stop_reason, extract_usage, part_message_text, part_thought_text, part_tool_call,
};

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

/**
 * Accumulates `streamGenerateContent?alt=sse` chunks. Each `data:` frame is a
 * full GenerateContentResponse with incremental candidate parts; there is no
 * terminal frame — the turn is complete at EOF once a `finishReason` has been
 * seen. The accumulated parts reconstruct a non-streaming response for the
 * ProviderMetadata event so thought-signature replay works on later turns.
 */
#[derive(Default)]
pub(crate) struct VertexStreamState {
    parts: Vec<Value>,
    response_id: Option<String>,
    model_version: Option<String>,
    finish_reason: Option<String>,
    usage: Option<Value>,
    emitted_tool_call: bool,
    blocked: bool,
}

impl VertexStreamState {
    /// Parses one SSE frame into inference events. An in-stream `error`
    /// payload surfaces as a terminal `Failed` event.
    pub(crate) fn push_frame(&mut self, frame: &str) -> anyhow::Result<Vec<InferenceEvent>> {
        let Some(data) = frame_data(frame) else {
            return Ok(Vec::new());
        };
        let chunk: Value = serde_json::from_str(&data)
            .map_err(|_| anyhow::anyhow!("Vertex AI stream sent an unparseable data frame"))?;
        if let Some(error) = chunk.get("error") {
            let status = error.get("status").and_then(Value::as_str).unwrap_or("");
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            return Ok(vec![InferenceEvent::Failed(InferenceFailure {
                message: format!("Vertex AI stream error ({status}): {message}"),
            })]);
        }
        Ok(self.push_chunk(&chunk))
    }

    fn push_chunk(&mut self, chunk: &Value) -> Vec<InferenceEvent> {
        let mut events = Vec::new();
        if let Some(block_reason) = chunk
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            // A pre-generation safety block arrives as a promptFeedback-only
            // frame with no candidates or finishReason. It is deterministic,
            // so surface it as a terminal Failed instead of letting EOF treat
            // the missing finishReason as retryable truncation.
            self.blocked = true;
            return vec![InferenceEvent::Failed(InferenceFailure {
                message: format!("Vertex AI blocked the prompt ({block_reason})"),
            })];
        }
        if let Some(id) = chunk.get("responseId").and_then(Value::as_str) {
            self.response_id = Some(id.to_string());
        }
        if let Some(version) = chunk.get("modelVersion").and_then(Value::as_str) {
            self.model_version = Some(version.to_string());
        }
        if let Some(usage) = chunk.get("usageMetadata") {
            self.usage = Some(usage.clone());
        }
        if let Some(reason) = chunk
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.to_string());
        }
        let parts = chunk
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for part in parts {
            if let Some(thought) = part_thought_text(&part) {
                if !thought.is_empty() {
                    events.push(InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: thought.to_string(),
                    }));
                }
            } else if let Some(text) = part_message_text(&part) {
                if !text.is_empty() {
                    events.push(InferenceEvent::MessageDelta(MessageDelta {
                        text: text.to_string(),
                        phase: None,
                    }));
                }
            } else if let Some(call) = part_tool_call(&part) {
                self.emitted_tool_call = true;
                events.push(InferenceEvent::ToolCallCompleted(call));
            }
            self.parts.push(part);
        }
        events
    }

    pub(crate) fn saw_finish_reason(&self) -> bool {
        self.finish_reason.is_some()
    }

    /// True once a pre-generation `promptFeedback.blockReason` was seen; the
    /// emitted `Failed` is deterministic and must not be retried.
    pub(crate) fn was_blocked(&self) -> bool {
        self.blocked
    }

    /// Terminal events emitted at EOF: usage, the reconstructed provider
    /// response, and completion metadata.
    pub(crate) fn finish(self) -> Vec<InferenceEvent> {
        let mut events = Vec::new();
        if let Some(usage) = self.usage.as_ref() {
            events.push(InferenceEvent::Usage(extract_usage(usage)));
        }
        let mut response = json!({
            "candidates": [{
                "content": { "role": "model", "parts": self.parts }
            }]
        });
        if let Some(reason) = self.finish_reason.as_ref() {
            response["candidates"][0]["finishReason"] = json!(reason);
        }
        if let Some(id) = self.response_id.as_ref() {
            response["responseId"] = json!(id);
        }
        if let Some(version) = self.model_version.as_ref() {
            response["modelVersion"] = json!(version);
        }
        if let Some(usage) = self.usage {
            response["usageMetadata"] = usage;
        }
        events.push(InferenceEvent::ProviderMetadata(response));
        events.push(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: self
                .finish_reason
                .map(|reason| canonical_stop_reason(&reason, self.emitted_tool_call)),
            provider_response_id: self.response_id,
        }));
        events
    }
}

/// Joins the `data:` lines of one SSE frame; None for comment/event-only
/// frames.
fn frame_data(frame: &str) -> Option<String> {
    let lines: Vec<&str> = frame
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data:")
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::{TokenUsage, ToolCallCompleted};

    fn frame(chunk: Value) -> String {
        format!("data: {chunk}")
    }

    #[test]
    fn accumulates_chunks_into_events_and_reconstructed_response() {
        let mut state = VertexStreamState::default();

        let first = state
            .push_frame(&frame(json!({
                "responseId": "resp_1",
                "modelVersion": "gemini-3.5-flash",
                "candidates": [{ "content": { "role": "model", "parts": [
                    { "text": "thinking hard", "thought": true },
                    { "text": "Hel" }
                ] } }]
            })))
            .unwrap();
        let second = state
            .push_frame(&frame(json!({
                "candidates": [{ "content": { "parts": [{ "text": "lo" }] } }]
            })))
            .unwrap();
        let third = state
            .push_frame(&frame(json!({
                "candidates": [{
                    "content": { "parts": [
                        { "functionCall": { "id": "call_1", "name": "shell", "args": { "cmd": "ls" } },
                          "thoughtSignature": "sig" }
                    ] },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 5,
                    "totalTokenCount": 15,
                    "cachedContentTokenCount": 4
                }
            })))
            .unwrap();

        assert_eq!(
            first,
            vec![
                InferenceEvent::ReasoningDelta(ReasoningDelta {
                    text: "thinking hard".to_string(),
                }),
                InferenceEvent::MessageDelta(MessageDelta {
                    text: "Hel".to_string(),
                    phase: None,
                }),
            ]
        );
        assert_eq!(
            second,
            vec![InferenceEvent::MessageDelta(MessageDelta {
                text: "lo".to_string(),
                phase: None,
            })]
        );
        assert_eq!(
            third,
            vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            })]
        );
        assert!(state.saw_finish_reason());

        let terminal = state.finish();
        assert_eq!(
            terminal[0],
            InferenceEvent::Usage(TokenUsage::new(10, 5, 15).with_cached_prompt_tokens(4))
        );
        let InferenceEvent::ProviderMetadata(metadata) = &terminal[1] else {
            panic!("expected ProviderMetadata, got {:?}", terminal[1]);
        };
        assert_eq!(metadata["responseId"], "resp_1");
        assert_eq!(metadata["candidates"][0]["finishReason"], "STOP");
        let parts = metadata["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[3]["thoughtSignature"], "sig");
        // STOP + emitted function call canonicalizes to tool_use.
        assert_eq!(
            terminal[2],
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("tool_use".to_string()),
                provider_response_id: Some("resp_1".to_string()),
            })
        );
    }

    #[test]
    fn error_payload_becomes_terminal_failed_event() {
        let mut state = VertexStreamState::default();

        let events = state
            .push_frame(&frame(json!({
                "error": { "code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Quota exceeded" }
            })))
            .unwrap();

        assert_eq!(
            events,
            vec![InferenceEvent::Failed(InferenceFailure {
                message: "Vertex AI stream error (RESOURCE_EXHAUSTED): Quota exceeded".to_string(),
            })]
        );
    }

    #[test]
    fn prompt_feedback_block_becomes_terminal_failed_event() {
        let mut state = VertexStreamState::default();

        let events = state
            .push_frame(&frame(
                json!({ "promptFeedback": { "blockReason": "SAFETY" } }),
            ))
            .unwrap();

        assert_eq!(
            events,
            vec![InferenceEvent::Failed(InferenceFailure {
                message: "Vertex AI blocked the prompt (SAFETY)".to_string(),
            })]
        );
        assert!(state.was_blocked());
        // A pre-generation block carries no finishReason; without the terminal
        // Failed above, EOF would misread that as retryable truncation.
        assert!(!state.saw_finish_reason());
    }

    #[test]
    fn unparseable_data_frame_is_an_error() {
        let mut state = VertexStreamState::default();
        assert!(state.push_frame("data: {truncated").is_err());
    }

    #[test]
    fn comment_and_event_only_frames_emit_nothing() {
        let mut state = VertexStreamState::default();
        assert_eq!(state.push_frame(": keep-alive").unwrap(), Vec::new());
        assert_eq!(state.push_frame("event: ping").unwrap(), Vec::new());
    }

    #[test]
    fn frame_buffer_reassembles_split_frames_and_multibyte_utf8() {
        let body = "data: {\"a\":\"héllo 🦀\"}\n\ndata: {\"b\":2}\r\n\r\n";
        let split = body.find('🦀').unwrap() + 2;
        let mut buffer = SseFrameBuffer::default();

        buffer.push(&body.as_bytes()[..split]);
        assert_eq!(buffer.take_frame(), None);
        buffer.push(&body.as_bytes()[split..]);

        assert_eq!(
            buffer.take_frame().as_deref(),
            Some("data: {\"a\":\"héllo 🦀\"}")
        );
        assert_eq!(buffer.take_frame().as_deref(), Some("data: {\"b\":2}"));
        assert_eq!(buffer.take_frame(), None);
        assert_eq!(buffer.take_trailing(), None);
    }

    #[test]
    fn frame_buffer_reports_frame_cap_overflow() {
        let mut buffer = SseFrameBuffer::default();
        // Undelimited bytes just under the cap stay within bounds; pushing past
        // it trips the guard so the driver can turn it into a stream error
        // instead of growing the buffer without bound.
        buffer.push(&vec![b'x'; MAX_FRAME_BYTES]);
        assert!(buffer.within_frame_cap());
        assert_eq!(buffer.take_frame(), None);
        buffer.push(b"y");
        assert!(!buffer.within_frame_cap());
    }

    #[test]
    fn frame_buffer_finds_delimiter_spanning_scanned_boundary() {
        let mut buffer = SseFrameBuffer::default();
        // First push ends mid-delimiter ("data: a\r\n"); the scanned offset
        // must rewind enough that the "\r\n\r\n" completed by the next push is
        // still found rather than skipped.
        buffer.push(b"data: a\r\n");
        assert_eq!(buffer.take_frame(), None);
        buffer.push(b"\r\ndata: b\r\n\r\n");
        assert_eq!(buffer.take_frame().as_deref(), Some("data: a"));
        assert_eq!(buffer.take_frame().as_deref(), Some("data: b"));
        assert_eq!(buffer.take_frame(), None);
    }

    #[test]
    fn finish_without_finish_reason_reports_no_stop_reason() {
        let mut state = VertexStreamState::default();
        state
            .push_frame(&frame(json!({
                "candidates": [{ "content": { "parts": [{ "text": "partial" }] } }]
            })))
            .unwrap();

        assert!(!state.saw_finish_reason());
    }
}
