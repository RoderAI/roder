//! Reading a whole Responses API event stream as one reply.
//!
//! The ChatGPT/Codex backend only answers `stream: true`, so the text
//! helper's single short reply arrives as server-sent events. The helper
//! needs nothing until the end, so the stream is read to its close and
//! reduced to the final `response` object, which carries the output and the
//! usage exactly as a non-streamed reply would. A terminal event whose
//! `output` is empty gets the items the stream announced one by one
//! (`response.output_item.done`), since a backend that stores nothing may
//! leave them out of it.

use serde_json::Value;

/// How a finished event stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamEnd {
    /// `response.completed` (or `response.incomplete`): its `response`.
    Response(Value),
    /// The provider reported a failure inside the stream (`response.failed`
    /// or an `error` event), with its code and message.
    Failed(String),
    /// The stream closed before any terminal event: cut off in transit.
    Truncated,
}

/// Whether a body without a content type is an event stream: its first
/// line is an SSE field (`event:`, `data:`, `id:` or a `:` comment).
pub(crate) fn looks_like(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |start| &bytes[start..]);
    [&b"event:"[..], b"data:", b"id:", b":"]
        .iter()
        .any(|field| start.starts_with(field))
}

/// The terminal event of a complete SSE body.
pub(crate) fn finish(bytes: &[u8]) -> StreamEnd {
    let text = String::from_utf8_lossy(bytes);
    let mut items = Vec::new();
    // Events are separated by a blank line; each `data:` line of one event
    // is joined with a newline, per the SSE spec.
    for event in text.split("\n\n").flat_map(|chunk| chunk.split("\r\n\r\n")) {
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|data| data.strip_prefix(' ').unwrap_or(data))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(parsed) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        match parsed["type"].as_str() {
            Some("response.output_item.done") => items.push(parsed["item"].clone()),
            Some("response.completed" | "response.incomplete" | "response.done") => {
                let mut response = parsed["response"].clone();
                let empty = response["output"].as_array().is_none_or(Vec::is_empty);
                if empty && !items.is_empty() && response.is_object() {
                    response["output"] = Value::Array(items);
                }
                return StreamEnd::Response(response);
            }
            Some("response.failed") => {
                return StreamEnd::Failed(failure(&parsed["response"]["error"]));
            }
            Some("error") => {
                let error = parsed.get("error").unwrap_or(&parsed);
                return StreamEnd::Failed(failure(error));
            }
            _ => {}
        }
    }
    StreamEnd::Truncated
}

/// `code: message`, from whichever the provider gave.
fn failure(error: &Value) -> String {
    let code = error["code"].as_str().or_else(|| error["type"].as_str());
    let message = error["message"].as_str().unwrap_or("no message");
    match code {
        Some(code) => format!("{code}: {message}"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_completed_event_carries_the_response() {
        let body = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"{\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":3}}}\n\n",
        );
        assert_eq!(
            finish(body.as_bytes()),
            StreamEnd::Response(json!({"status": "completed", "usage": {"input_tokens": 3}}))
        );
    }

    #[test]
    fn streamed_items_fill_an_empty_final_output() {
        let item = json!({"type": "message", "content": [{"type": "output_text", "text": "hi"}]});
        let body = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type": "response.output_item.done", "item": item}),
            json!({"type": "response.completed", "response": {"output": [], "usage": {}}}),
        );
        assert_eq!(
            finish(body.as_bytes()),
            StreamEnd::Response(json!({"output": [item], "usage": {}}))
        );
    }

    #[test]
    fn a_failure_in_the_stream_keeps_its_code_and_message() {
        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"slow down\"}}}\n\n";
        assert_eq!(
            finish(failed.as_bytes()),
            StreamEnd::Failed("rate_limit_exceeded: slow down".into())
        );
        let error = "data: {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"bad\"}}\r\n\r\n";
        assert_eq!(
            finish(error.as_bytes()),
            StreamEnd::Failed("invalid_request_error: bad".into())
        );
    }

    #[test]
    fn a_stream_is_known_by_its_first_field() {
        assert!(looks_like(b"event: response.created\ndata: {}\n\n"));
        assert!(looks_like(b"\ndata: {}"));
        assert!(!looks_like(br#"{"choices":[]}"#));
        assert!(!looks_like(b"<html>"));
        assert!(!looks_like(b""));
    }

    #[test]
    fn a_stream_without_a_terminal_event_was_cut_off() {
        let body = "data: {\"type\":\"response.created\",\"response\":{}}\n\ndata: {\"type\":\"response.output";
        assert_eq!(finish(body.as_bytes()), StreamEnd::Truncated);
        assert_eq!(finish(b""), StreamEnd::Truncated);
    }
}
