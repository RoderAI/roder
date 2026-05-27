use std::collections::BTreeMap;

use serde_json::json;

const CONNECT_COMPRESSED_FLAG: u8 = 1;
const CONNECT_END_STREAM_FLAG: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAgentMessage {
    pub text: String,
    pub thinking: String,
    pub usage_fields: BTreeMap<u32, u64>,
    pub tool_calls: Vec<CursorToolCall>,
    pub turn_ended: bool,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectFrame {
    Payload(Vec<u8>),
    EndStream(Option<String>),
}

#[derive(Debug, Clone)]
enum ProtoValue {
    Varint(u64),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
struct ProtoField {
    no: u32,
    value: ProtoValue,
}

pub fn encode_agent_client_message(
    prompt: &str,
    model_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        1,
        encode_agent_run_request(prompt, model_id, conversation_id, message_id),
    )])
}

fn encode_agent_run_request(
    prompt: &str,
    model_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> Vec<u8> {
    proto_message(vec![
        proto_field_bytes(1, Vec::new()),
        proto_field_bytes(2, encode_conversation_action(prompt, message_id)),
        proto_field_bytes(4, Vec::new()),
        proto_field_string(5, conversation_id),
        proto_field_bytes(9, encode_requested_model(model_id)),
        proto_field_varint(12, 0),
        proto_field_string(16, conversation_id),
    ])
}

fn encode_conversation_action(prompt: &str, message_id: &str) -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![
                proto_field_string(1, prompt),
                proto_field_string(2, message_id),
                proto_field_bytes(3, Vec::new()),
                proto_field_varint(4, 2),
            ]),
        )]),
    )])
}

fn encode_requested_model(model_id: &str) -> Vec<u8> {
    proto_message(vec![
        proto_field_string(1, model_id),
        proto_field_bytes(
            3,
            proto_message(vec![
                proto_field_string(1, "fast"),
                proto_field_string(2, "false"),
            ]),
        ),
    ])
}

pub fn encode_connect_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(0);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn encode_cli_stream_control_frames() -> Vec<Vec<u8>> {
    [
        "2a020a00",
        "1a021a00",
        "1a0408011a00",
        "1a0408021a00",
        "1a0408031a00",
        "1a0408051a00",
        "1a0408041a00",
        "1a0408061a00",
        "1a0408071a00",
    ]
    .into_iter()
    .map(hex_to_bytes)
    .map(|payload| encode_connect_frame(&payload))
    .collect()
}

pub fn take_connect_frame(buffer: &mut Vec<u8>) -> anyhow::Result<Option<ConnectFrame>> {
    if buffer.len() < 5 {
        return Ok(None);
    }
    let flags = buffer[0];
    let len = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;
    if buffer.len() < 5 + len {
        return Ok(None);
    }
    let payload = buffer[5..5 + len].to_vec();
    buffer.drain(..5 + len);
    if flags & CONNECT_COMPRESSED_FLAG == CONNECT_COMPRESSED_FLAG {
        anyhow::bail!("Cursor returned compressed Connect payload; compression is not enabled");
    }
    if flags & CONNECT_END_STREAM_FLAG == CONNECT_END_STREAM_FLAG {
        return Ok(Some(ConnectFrame::EndStream(parse_end_stream(&payload))));
    }
    Ok(Some(ConnectFrame::Payload(payload)))
}

pub fn is_context_frame(frame: &[u8]) -> bool {
    if frame.len() < 5 || frame[0] != 0 {
        return false;
    }
    let len = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
    if frame.len() < 5 + len {
        return false;
    }
    decode_fields_safe(&frame[5..5 + len])
        .into_iter()
        .any(|field| field.no == 2 && matches!(field.value, ProtoValue::Bytes(_)))
}

pub fn decode_agent_server_message(payload: &[u8]) -> DecodedAgentMessage {
    let mut result = DecodedAgentMessage {
        text: String::new(),
        thinking: String::new(),
        usage_fields: BTreeMap::new(),
        tool_calls: Vec::new(),
        turn_ended: false,
        strings: collect_utf8_strings(payload, 0),
    };
    for field in decode_fields_safe(payload) {
        let ProtoValue::Bytes(bytes) = field.value else {
            continue;
        };
        if field.no == 1 {
            merge_interaction_update(&mut result, &bytes);
        } else if field.no == 2 {
            merge_legacy_update(&mut result, &bytes);
        }
    }
    result
}

pub(crate) fn proto_message(parts: Vec<Vec<u8>>) -> Vec<u8> {
    let size = parts.iter().map(Vec::len).sum();
    let mut output = Vec::with_capacity(size);
    for part in parts {
        output.extend_from_slice(&part);
    }
    output
}

pub(crate) fn proto_field_string(no: u32, value: &str) -> Vec<u8> {
    proto_field_bytes(no, value.as_bytes().to_vec())
}

pub(crate) fn proto_field_bytes(no: u32, bytes: Vec<u8>) -> Vec<u8> {
    let mut out = encode_varint(((no << 3) | 2) as u64);
    out.extend_from_slice(&encode_varint(bytes.len() as u64));
    out.extend_from_slice(&bytes);
    out
}

pub(crate) fn proto_field_varint(no: u32, value: u64) -> Vec<u8> {
    let mut out = encode_varint((no << 3) as u64);
    out.extend_from_slice(&encode_varint(value));
    out
}

fn merge_interaction_update(result: &mut DecodedAgentMessage, bytes: &[u8]) {
    for field in decode_fields_safe(bytes) {
        let ProtoValue::Bytes(value) = field.value else {
            continue;
        };
        if field.no == 1 {
            result.text.push_str(&decode_text_field(&value));
        } else if field.no == 2 {
            if let Some(call) = decode_cursor_tool_call(&value) {
                result.tool_calls.push(call);
            }
        } else if field.no == 4 {
            result.thinking.push_str(&decode_text_field(&value));
        } else if field.no == 14 {
            result.usage_fields.extend(decode_usage(&value));
            result.turn_ended = true;
        }
    }
}

fn decode_cursor_tool_call(bytes: &[u8]) -> Option<CursorToolCall> {
    let fields = decode_fields_safe(bytes);
    let id = fields
        .iter()
        .find_map(|field| bytes_field_as_string(field, 1))
        .filter(|value| value.starts_with("tool_"))?;
    let payload = fields
        .iter()
        .find_map(|field| bytes_field(field, 2))
        .cloned()
        .unwrap_or_default();
    if let Some(path) =
        nested_string(&payload, &[8, 1, 1]).or_else(|| path_candidate_from_strings(&payload))
    {
        return Some(CursorToolCall {
            id,
            name: "read_file".to_string(),
            arguments: json!({ "path": path }).to_string(),
        });
    }

    let strings = collect_utf8_strings(&payload, 0)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    Some(CursorToolCall {
        id,
        name: "cursor_unsupported_tool".to_string(),
        arguments: json!({
            "reason": "unsupported_cursor_native_tool",
            "strings": strings,
        })
        .to_string(),
    })
}

fn bytes_field<'a>(field: &'a ProtoField, no: u32) -> Option<&'a Vec<u8>> {
    match &field.value {
        ProtoValue::Bytes(value) if field.no == no => Some(value),
        _ => None,
    }
}

fn bytes_field_as_string(field: &ProtoField, no: u32) -> Option<String> {
    bytes_field(field, no).and_then(|value| {
        let text = String::from_utf8_lossy(value).to_string();
        looks_like_text(&text).then_some(text)
    })
}

fn nested_string(bytes: &[u8], path: &[u32]) -> Option<String> {
    let Some((first, rest)) = path.split_first() else {
        return None;
    };
    for field in decode_fields_safe(bytes) {
        let Some(value) = bytes_field(&field, *first) else {
            continue;
        };
        if rest.is_empty() {
            let text = String::from_utf8_lossy(value).to_string();
            if looks_like_text(&text) {
                return Some(text);
            }
        } else if let Some(text) = nested_string(value, rest) {
            return Some(text);
        }
    }
    None
}

fn path_candidate_from_strings(bytes: &[u8]) -> Option<String> {
    collect_utf8_strings(bytes, 0)
        .into_iter()
        .flat_map(|value| {
            value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .find(|value| {
            (value.starts_with('/') || value.starts_with("./") || value.starts_with("../"))
                && !value.starts_with("tool_")
        })
}

fn merge_legacy_update(result: &mut DecodedAgentMessage, bytes: &[u8]) {
    for field in decode_fields_safe(bytes) {
        if let ProtoValue::Bytes(value) = field.value {
            if field.no == 1 {
                result.text.push_str(&String::from_utf8_lossy(&value));
            } else if field.no == 25 {
                result.thinking.push_str(&decode_text_field(&value));
            }
        }
    }
}

fn decode_text_field(bytes: &[u8]) -> String {
    decode_fields_safe(bytes)
        .into_iter()
        .filter_map(|field| match field.value {
            ProtoValue::Bytes(value) if field.no == 1 => {
                Some(String::from_utf8_lossy(&value).to_string())
            }
            _ => None,
        })
        .collect()
}

fn decode_usage(bytes: &[u8]) -> BTreeMap<u32, u64> {
    decode_fields_safe(bytes)
        .into_iter()
        .filter_map(|field| match field.value {
            ProtoValue::Varint(value) => Some((field.no, value)),
            _ => None,
        })
        .collect()
}

fn collect_utf8_strings(bytes: &[u8], depth: u8) -> Vec<String> {
    if depth > 5 {
        return Vec::new();
    }
    let mut values = Vec::new();
    for field in decode_fields_safe(bytes) {
        let ProtoValue::Bytes(value) = field.value else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(&value).to_string();
        if looks_like_text(&text) {
            values.push(text);
        }
        values.extend(collect_utf8_strings(&value, depth + 1));
    }
    values
}

fn looks_like_text(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let printable = trimmed
        .chars()
        .filter(|c| c.is_ascii_graphic() || c.is_ascii_whitespace())
        .count();
    printable * 10 >= trimmed.chars().count() * 9
}

fn parse_end_stream(payload: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(payload).trim().to_string();
    (!text.is_empty() && text != "{}").then_some(text)
}

fn decode_fields_safe(bytes: &[u8]) -> Vec<ProtoField> {
    decode_fields(bytes).unwrap_or_default()
}

fn decode_fields(bytes: &[u8]) -> anyhow::Result<Vec<ProtoField>> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (tag, next) = read_varint(bytes, offset)?;
        offset = next;
        let no = (tag >> 3) as u32;
        let wt = (tag & 7) as u8;
        match wt {
            0 => {
                let (value, next) = read_varint(bytes, offset)?;
                offset = next;
                fields.push(ProtoField {
                    no,
                    value: ProtoValue::Varint(value),
                });
            }
            2 => {
                let (len, next) = read_varint(bytes, offset)?;
                offset = next;
                let end = offset + len as usize;
                if end > bytes.len() {
                    anyhow::bail!("protobuf length-delimited field exceeds payload size");
                }
                fields.push(ProtoField {
                    no,
                    value: ProtoValue::Bytes(bytes[offset..end].to_vec()),
                });
                offset = end;
            }
            1 => offset += 8,
            5 => offset += 4,
            other => anyhow::bail!("unsupported protobuf wire type {other}"),
        }
    }
    Ok(fields)
}

fn read_varint(bytes: &[u8], mut offset: usize) -> anyhow::Result<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    while offset < bytes.len() {
        let byte = bytes[offset];
        offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
        shift += 7;
    }
    anyhow::bail!("unexpected end of protobuf varint")
}

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value >= 0x80 {
        bytes.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_frame_round_trips_payload() {
        let mut frame = encode_connect_frame(b"abc");
        let parsed = take_connect_frame(&mut frame).unwrap();
        assert_eq!(parsed, Some(ConnectFrame::Payload(b"abc".to_vec())));
        assert!(frame.is_empty());
    }

    #[test]
    fn context_frame_detection_checks_payload_field() {
        let frame = encode_connect_frame(&proto_message(vec![proto_field_bytes(
            2,
            proto_message(vec![proto_field_string(1, "context")]),
        )]));
        assert!(is_context_frame(&frame));
        assert!(!is_context_frame(&encode_connect_frame(b"abc")));
    }

    #[test]
    fn agent_client_message_contains_prompt_and_model() {
        let bytes = encode_agent_client_message("hello cursor", "composer-2.5", "conv", "msg");
        let strings = collect_utf8_strings(&bytes, 0);
        assert!(strings.iter().any(|value| value.contains("hello cursor")));
        assert!(strings.iter().any(|value| value.contains("composer-2.5")));
    }

    #[test]
    fn decodes_cursor_read_tool_call_from_interaction_update() {
        let payload = proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_bytes(
                2,
                proto_message(vec![
                    proto_field_string(1, "tool_read_123"),
                    proto_field_bytes(
                        2,
                        proto_message(vec![proto_field_bytes(
                            8,
                            proto_message(vec![proto_field_bytes(
                                1,
                                proto_message(vec![proto_field_string(1, "AGENTS.md")]),
                            )]),
                        )]),
                    ),
                    proto_field_string(3, "model-call-0"),
                ]),
            )]),
        )]);

        let decoded = decode_agent_server_message(&payload);

        assert_eq!(
            decoded.tool_calls,
            vec![CursorToolCall {
                id: "tool_read_123".to_string(),
                name: "read_file".to_string(),
                arguments: r#"{"path":"AGENTS.md"}"#.to_string(),
            }]
        );
    }

    #[test]
    fn decodes_unknown_cursor_tool_call_as_safe_unsupported_tool() {
        let payload = proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_bytes(
                2,
                proto_message(vec![
                    proto_field_string(1, "tool_unknown_123"),
                    proto_field_bytes(2, proto_message(vec![proto_field_string(9, "mystery")])),
                ]),
            )]),
        )]);

        let decoded = decode_agent_server_message(&payload);

        assert_eq!(decoded.tool_calls.len(), 1);
        assert_eq!(decoded.tool_calls[0].id, "tool_unknown_123");
        assert_eq!(decoded.tool_calls[0].name, "cursor_unsupported_tool");
        assert!(
            decoded.tool_calls[0]
                .arguments
                .contains("unsupported_cursor_native_tool")
        );
    }
}
