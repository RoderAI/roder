//! Message parser for Claude CLI SSE streaming output.
//!
//! Parses server-sent events (SSE) format with JSON payloads containing
//! Claude CLI message types like start, token, message_stop, and error events.

use crate::error::{ClaudeSDKError, MessageParseError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Lines};

/// Represents a parsed SSE line type
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedLine {
    /// Empty line (just whitespace)
    Empty,
    /// Event type line: `event: <name>`
    Event(String),
    /// Data line with JSON payload: `data: <json>`
    Data(String),
    /// Unknown line format
    Unknown(String),
}

/// Represents a parsed stream event from the Claude CLI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Stream start event with UUID
    Start {
        uuid: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// Token/stream chunk with text content
    Token {
        index: u32,
        token: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
    },
    /// Message stop event indicating end of response
    MessageStop {
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
    },
    /// Error event from the CLI
    Error { error: StreamError },
}

/// Error details within a stream error event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Raw SSE event before JSON parsing
#[derive(Debug, Default)]
struct RawSseEvent {
    event_type: Option<String>,
    data: Option<String>,
}

/// Parse a single SSE line into a `ParsedLine`
///
/// Handles:
/// - Empty/whitespace lines -> `ParsedLine::Empty`
/// - Event type lines: `event: <name>` -> `ParsedLine::Event(name)`
/// - Data lines: `data: <json>` -> `ParsedLine::Data(json)`
/// - Everything else -> `ParsedLine::Unknown(line)`
pub fn parse_line(line: &str) -> Result<ParsedLine, ClaudeSDKError> {
    let trimmed = line.trim_end();

    if trimmed.is_empty() {
        return Ok(ParsedLine::Empty);
    }

    if let Some(event_value) = trimmed.strip_prefix("event: ") {
        return Ok(ParsedLine::Event(event_value.to_string()));
    }

    if let Some(data_value) = trimmed.strip_prefix("data: ") {
        return Ok(ParsedLine::Data(data_value.to_string()));
    }

    Ok(ParsedLine::Unknown(trimmed.to_string()))
}

/// Parse an SSE stream into a iterator of `StreamEvent`s
///
/// Reads lines from the provided reader, accumulates SSE events
/// (event type + data lines), and parses the JSON data into typed events.
///
/// # Example
///
/// ```rust,ignore
/// use std::io::BufReader;
/// use claude_code_sdk_rust::internal::message_parser::parse_sse_stream;
///
/// let reader = BufReader::new(stream);
/// for event in parse_sse_stream(reader) {
///     match event {
///         Ok(StreamEvent::Token { token, .. }) => print!("{}", token),
///         Ok(StreamEvent::MessageStop { .. }) => break,
///         Ok(StreamEvent::Error { error }) => eprintln!("Error: {}", error.message),
///         _ => {}
///     }
/// }
/// ```
pub fn parse_sse_stream<R: BufRead>(
    reader: R,
) -> impl Iterator<Item = Result<StreamEvent, ClaudeSDKError>> {
    SseStreamIterator {
        lines: reader.lines(),
        current_event: RawSseEvent::default(),
    }
}

/// Iterator over SSE stream events
struct SseStreamIterator<R: BufRead> {
    lines: Lines<R>,
    current_event: RawSseEvent,
}

impl<R: BufRead> Iterator for SseStreamIterator<R> {
    type Item = Result<StreamEvent, ClaudeSDKError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.lines.next() {
                Some(Ok(line)) => {
                    match parse_line(&line) {
                        Ok(ParsedLine::Empty) => {
                            // Empty line indicates end of an SSE event
                            if self.current_event.data.is_some() {
                                let event = self.flush_event();
                                return Some(event);
                            }
                            // Otherwise just continue reading
                        }
                        Ok(ParsedLine::Event(event_type)) => {
                            self.current_event.event_type = Some(event_type);
                        }
                        Ok(ParsedLine::Data(data)) => {
                            self.current_event.data = Some(data);
                        }
                        Ok(ParsedLine::Unknown(_)) => {
                            // Skip unknown lines
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
                Some(Err(e)) => {
                    return Some(Err(ClaudeSDKError::IO(e)));
                }
                None => {
                    // End of stream - flush any pending event
                    if self.current_event.data.is_some() {
                        let event = self.flush_event();
                        return Some(event);
                    }
                    return None;
                }
            }
        }
    }
}

impl<R: BufRead> SseStreamIterator<R> {
    /// Parse and return the current accumulated event, then reset
    fn flush_event(&mut self) -> Result<StreamEvent, ClaudeSDKError> {
        let data = self.current_event.data.take().unwrap_or_default();
        let result = parse_stream_event(&data);
        self.current_event = RawSseEvent::default();
        result
    }
}

/// Parse a JSON string into a `StreamEvent`
fn parse_stream_event(json_str: &str) -> Result<StreamEvent, ClaudeSDKError> {
    let value: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
        ClaudeSDKError::CLIJSONDecode(crate::error::CLIJSONDecodeError::new(json_str, e))
    })?;

    let event_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MessageParseError::new("Stream event missing 'type' field"))?;

    match event_type {
        "start" => parse_start_event(value),
        "token" => parse_token_event(value),
        "message_stop" => parse_message_stop_event(value),
        "error" => parse_error_event(value),
        _ => {
            Err(MessageParseError::new(format!("Unknown stream event type: {}", event_type)).into())
        }
    }
}

fn parse_start_event(value: serde_json::Value) -> Result<StreamEvent, ClaudeSDKError> {
    let uuid = value
        .get("uuid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MessageParseError::new("Start event missing 'uuid' field"))?;

    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(StreamEvent::Start {
        uuid: uuid.to_string(),
        version,
    })
}

fn parse_token_event(value: serde_json::Value) -> Result<StreamEvent, ClaudeSDKError> {
    let index = value
        .get("index")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .ok_or_else(|| MessageParseError::new("Token event missing 'index' field"))?;

    let token = value
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MessageParseError::new("Token event missing 'token' field"))?;

    let stop_reason = value
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(StreamEvent::Token {
        index,
        token: token.to_string(),
        stop_reason,
    })
}

fn parse_message_stop_event(value: serde_json::Value) -> Result<StreamEvent, ClaudeSDKError> {
    let stop_reason = value
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(StreamEvent::MessageStop { stop_reason })
}

fn parse_error_event(value: serde_json::Value) -> Result<StreamEvent, ClaudeSDKError> {
    let error_obj = value
        .get("error")
        .ok_or_else(|| MessageParseError::new("Error event missing 'error' field"))?;

    let message = error_obj
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MessageParseError::new("Error object missing 'message' field"))?
        .to_string();

    let code = error_obj
        .get("code")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(StreamEvent::Error {
        error: StreamError { message, code },
    })
}

/// Parse a raw JSON string into a JSON Value, returning a structured error on failure
pub fn parse_json_line(line: &str) -> Result<serde_json::Value, ClaudeSDKError> {
    serde_json::from_str(line)
        .map_err(|e| ClaudeSDKError::CLIJSONDecode(crate::error::CLIJSONDecodeError::new(line, e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_line_empty() {
        assert_eq!(parse_line("").unwrap(), ParsedLine::Empty);
        assert_eq!(parse_line("   ").unwrap(), ParsedLine::Empty);
        assert_eq!(parse_line("\n").unwrap(), ParsedLine::Empty);
    }

    #[test]
    fn test_parse_line_event() {
        assert_eq!(
            parse_line("event: start").unwrap(),
            ParsedLine::Event("start".to_string())
        );
        assert_eq!(
            parse_line("event: token").unwrap(),
            ParsedLine::Event("token".to_string())
        );
    }

    #[test]
    fn test_parse_line_data() {
        assert_eq!(
            parse_line("data: {\"type\": \"start\"}").unwrap(),
            ParsedLine::Data("{\"type\": \"start\"}".to_string())
        );
    }

    #[test]
    fn test_parse_line_unknown() {
        assert_eq!(
            parse_line("random line").unwrap(),
            ParsedLine::Unknown("random line".to_string())
        );
    }

    #[test]
    fn test_parse_start_event() {
        let json = r#"{"type": "start", "uuid": "abc-123", "version": "1.0"}"#;
        let event = parse_stream_event(json).unwrap();

        match event {
            StreamEvent::Start { uuid, version } => {
                assert_eq!(uuid, "abc-123");
                assert_eq!(version, Some("1.0".to_string()));
            }
            _ => panic!("Expected Start event"),
        }
    }

    #[test]
    fn test_parse_token_event() {
        let json = r#"{"type": "token", "index": 0, "token": "Hello", "stop_reason": null}"#;
        let event = parse_stream_event(json).unwrap();

        match event {
            StreamEvent::Token {
                index,
                token,
                stop_reason,
            } => {
                assert_eq!(index, 0);
                assert_eq!(token, "Hello");
                assert_eq!(stop_reason, None);
            }
            _ => panic!("Expected Token event"),
        }
    }

    #[test]
    fn test_parse_message_stop_event() {
        let json = r#"{"type": "message_stop", "stop_reason": "end_turn"}"#;
        let event = parse_stream_event(json).unwrap();

        match event {
            StreamEvent::MessageStop { stop_reason } => {
                assert_eq!(stop_reason, Some("end_turn".to_string()));
            }
            _ => panic!("Expected MessageStop event"),
        }
    }

    #[test]
    fn test_parse_error_event() {
        let json =
            r#"{"type": "error", "error": {"message": "Something went wrong", "code": "E001"}}"#;
        let event = parse_stream_event(json).unwrap();

        match event {
            StreamEvent::Error { error } => {
                assert_eq!(error.message, "Something went wrong");
                assert_eq!(error.code, Some("E001".to_string()));
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_parse_sse_stream() {
        let sse_data = r#"event: start
data: {"type": "start", "uuid": "test-uuid"}

event: token
data: {"type": "token", "index": 0, "token": "Hi"}

event: message_stop
data: {"type": "message_stop", "stop_reason": "end_turn"}
"#;

        let cursor = Cursor::new(sse_data);
        let events: Vec<StreamEvent> = parse_sse_stream(cursor).filter_map(|r| r.ok()).collect();

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], StreamEvent::Start { .. }));
        assert!(matches!(events[1], StreamEvent::Token { .. }));
        assert!(matches!(events[2], StreamEvent::MessageStop { .. }));
    }
}
