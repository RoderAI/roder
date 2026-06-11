// Cursor `agent.v1` hand-rolled protobuf encoders/decoders.
//
// The wire schema (message shapes + field numbers) is documented in
// `../proto/agent_v1.proto`, transcribed from the Cursor app bundle's
// protobuf-es output. That `.proto` is reference/codegen material only — this
// module encodes by hand. Keep the two in sync: every field number used here
// should have a matching definition in `agent_v1.proto`.

use std::collections::BTreeMap;

use serde_json::{Value, json};

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

/// Encode an `agent.v1` client message, carrying prior turns (including
/// assistant tool calls and tool results) as native `agent.v1.ConversationHistory`.
/// This lets Cursor continue an agentic loop across Roder rounds: the model
/// sees the tool calls it already made and their results, so it progresses
/// (e.g. read -> edit) instead of re-issuing the same call. Pass an empty
/// `history` for a fresh turn.
pub fn encode_agent_client_message_with_history(
    prompt: &str,
    model_id: &str,
    conversation_id: &str,
    message_id: &str,
    history: &[CursorHistoryMessage],
    images: &[CursorImage],
    tools: &[CursorMcpTool],
) -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        1,
        encode_agent_run_request(
            prompt,
            model_id,
            conversation_id,
            message_id,
            history,
            images,
            tools,
        ),
    )])
}

fn encode_agent_run_request(
    prompt: &str,
    model_id: &str,
    conversation_id: &str,
    message_id: &str,
    history: &[CursorHistoryMessage],
    images: &[CursorImage],
    tools: &[CursorMcpTool],
) -> Vec<u8> {
    // Whether any inline image bytes are present in this turn (current message
    // or replayed history). Cursor only honours inline `SelectedImage.data` /
    // `ConversationHistoryImageContent` when the client advertises support via
    // `AgentRunRequest.client_supports_inline_images` (field 19).
    let has_inline_images = !images.is_empty()
        || history.iter().any(|item| {
            matches!(item, CursorHistoryMessage::User { images, .. } if !images.is_empty())
        });
    let mut fields = vec![
        proto_field_bytes(1, Vec::new()),
        proto_field_bytes(
            2,
            encode_conversation_action(prompt, message_id, history, images),
        ),
        // agent.v1.AgentRunRequest.mcp_tools (field 4) = agent.v1.McpTools.
        // Advertises Roder's tools to the Cursor model. Empty when Roder sends
        // no tools, which matches the prior empty-message encoding.
        proto_field_bytes(4, encode_mcp_tools(tools)),
        proto_field_string(5, conversation_id),
        proto_field_bytes(9, encode_requested_model(model_id)),
        proto_field_varint(12, 0),
        proto_field_string(16, conversation_id),
    ];
    if has_inline_images {
        // agent.v1.AgentRunRequest.client_supports_inline_images (field 19).
        fields.push(proto_field_varint(19, 1));
    }
    proto_message(fields)
}

/// `agent.v1.AgentMode` enum value enabling Cursor's full agentic tool loop.
/// (UNSPECIFIED=0, AGENT=1, ASK=2, PLAN=3, ...). Sourced from the Cursor app
/// bundle's `agent.v1` protobuf schema.
const AGENT_MODE_AGENT: u64 = 1;

/// A Roder tool advertised to the Cursor model via `AgentRunRequest.mcp_tools`.
///
/// Field numbers below are from the Cursor app bundle's `agent.v1` schema
/// (`cursor-agent-worker/dist/main.js`):
/// - `agent.v1.McpTools { mcp_tools 1: repeated McpToolDefinition }`
/// - `agent.v1.McpToolDefinition { name 1, description 2, input_schema 3 }`
///   where `input_schema` is a `google.protobuf.Value` (JSON Schema as a Value).
#[derive(Debug, Clone, PartialEq)]
pub struct CursorMcpTool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool input (Roder `ToolSpec.parameters`).
    pub input_schema: serde_json::Value,
}

/// Encode `agent.v1.McpTools { mcp_tools 1: repeated McpToolDefinition }`.
/// Returns an empty message when there are no tools (advertises nothing),
/// matching the historical empty-bytes encoding of `AgentRunRequest` field 4.
pub(crate) fn encode_mcp_tools(tools: &[CursorMcpTool]) -> Vec<u8> {
    proto_message(
        tools
            .iter()
            .map(|tool| proto_field_bytes(1, encode_mcp_tool_definition(tool)))
            .collect(),
    )
}

/// Encode `agent.v1.McpToolDefinition { name 1, description 2, input_schema 3 }`.
fn encode_mcp_tool_definition(tool: &CursorMcpTool) -> Vec<u8> {
    proto_message(vec![
        proto_field_string(1, &tool.name),
        proto_field_string(2, &tool.description),
        // input_schema is a google.protobuf.Value (the JSON Schema object).
        proto_field_bytes(3, encode_protobuf_value(&tool.input_schema)),
    ])
}

/// Encode a `serde_json::Value` as a `google.protobuf.Value` message body.
///
/// `google.protobuf.Value` is a oneof:
///   null_value 1 (enum, 0) · number_value 2 (double) · string_value 3 ·
///   bool_value 4 · struct_value 5 (Struct) · list_value 6 (ListValue).
fn encode_protobuf_value(value: &serde_json::Value) -> Vec<u8> {
    match value {
        serde_json::Value::Null => proto_field_varint(1, 0),
        serde_json::Value::Bool(b) => proto_field_varint(4, u64::from(*b)),
        serde_json::Value::Number(n) => {
            proto_field_double(2, n.as_f64().unwrap_or(0.0))
        }
        serde_json::Value::String(s) => proto_field_string(3, s),
        serde_json::Value::Object(_) => proto_field_bytes(5, encode_protobuf_struct(value)),
        serde_json::Value::Array(items) => {
            // google.protobuf.ListValue { values 1: repeated Value }.
            let list = proto_message(
                items
                    .iter()
                    .map(|item| proto_field_bytes(1, encode_protobuf_value(item)))
                    .collect(),
            );
            proto_field_bytes(6, list)
        }
    }
}

/// Encode a JSON object as a `google.protobuf.Struct { fields 1: map<string,Value> }`.
/// A proto3 map entry is a sub-message `{ key 1: string, value 2: Value }`.
fn encode_protobuf_struct(value: &serde_json::Value) -> Vec<u8> {
    let serde_json::Value::Object(map) = value else {
        return Vec::new();
    };
    proto_message(
        map.iter()
            .map(|(key, val)| {
                proto_field_bytes(
                    1,
                    proto_message(vec![
                        proto_field_string(1, key),
                        proto_field_bytes(2, encode_protobuf_value(val)),
                    ]),
                )
            })
            .collect(),
    )
}

/// A decoded inline image ready for the Cursor `agent.v1` wire format.
///
/// `data` is the raw (base64-decoded) image bytes and `mime_type` is the
/// source MIME type (e.g. `image/png`). Field numbers for the surrounding
/// messages are sourced from the Cursor app bundle's `agent.v1` protobuf
/// schema (`cursor-agent-worker/dist/main.js`):
/// - `agent.v1.SelectedImage { data 8: bytes, mime_type 7: string }`
/// - `agent.v1.ConversationHistoryImageContent { data 1: string, mime_type 2: string }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorImage {
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// One prior conversation turn, mapped to `agent.v1.ConversationHistoryMessage`.
#[derive(Debug, Clone)]
pub enum CursorHistoryMessage {
    User {
        text: String,
        images: Vec<CursorImage>,
    },
    AssistantText(String),
    AssistantToolCall {
        id: String,
        name: String,
        args_json: String,
    },
    ToolResult {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
}

fn encode_conversation_action(
    prompt: &str,
    message_id: &str,
    history: &[CursorHistoryMessage],
    images: &[CursorImage],
) -> Vec<u8> {
    // agent.v1.UserMessage { text 1, message_id 2, selected_context 3, mode 4 }
    let user_message = proto_message(vec![
        proto_field_string(1, prompt),
        proto_field_string(2, message_id),
        // agent.v1.UserMessage.selected_context (field 3) = agent.v1.SelectedContext.
        // Attached images ride here as `selected_images`; with no images this is a
        // zero-length message, matching the prior empty-bytes encoding.
        proto_field_bytes(3, encode_selected_context(images)),
        // agent.v1.UserMessage.mode (field 4) = agent.v1.AgentMode enum.
        // AGENT_MODE_AGENT = 1 enables Cursor's agentic tool loop (file edits,
        // shell, search). The previous value 2 = AGENT_MODE_ASK ran the model
        // read-only, so it refused edits ("Ask mode").
        proto_field_varint(4, AGENT_MODE_AGENT),
    ]);
    // agent.v1.UserMessageAction { user_message 1, conversation_history 7 }
    let mut user_message_action = vec![proto_field_bytes(1, user_message)];
    if !history.is_empty() {
        user_message_action.push(proto_field_bytes(7, encode_conversation_history(history)));
    }
    // agent.v1.ConversationAction { user_message_action 1 }
    proto_message(vec![proto_field_bytes(
        1,
        proto_message(user_message_action),
    )])
}

impl CursorImage {
    /// Parse a `data:<mime>;base64,<payload>` URL into raw bytes + MIME type.
    /// Returns `None` for non-base64 data URLs or malformed payloads (e.g. a
    /// remote `https://` image URL Cursor's inline path cannot carry).
    pub fn from_data_url(image_url: &str) -> Option<Self> {
        use base64::Engine as _;
        let rest = image_url.strip_prefix("data:")?;
        let (meta, payload) = rest.split_once(',')?;
        let meta = meta.strip_suffix(";base64")?;
        let mime_type = if meta.is_empty() {
            "application/octet-stream".to_string()
        } else {
            meta.to_string()
        };
        let data = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        Some(Self { mime_type, data })
    }
}

/// Encode `agent.v1.SelectedContext { selected_images 1: repeated SelectedImage }`.
/// Returns an empty message when there are no images so the field stays present
/// but carries no payload.
fn encode_selected_context(images: &[CursorImage]) -> Vec<u8> {
    if images.is_empty() {
        return Vec::new();
    }
    let fields = images
        .iter()
        .map(|image| proto_field_bytes(1, encode_selected_image(image)))
        .collect();
    proto_message(fields)
}

/// Encode `agent.v1.SelectedImage { data 8: bytes, mime_type 7: string }`.
/// `data` (field 8) is the inline-bytes arm of the `data_or_blob_id` oneof.
fn encode_selected_image(image: &CursorImage) -> Vec<u8> {
    proto_message(vec![
        proto_field_string(7, &image.mime_type),
        proto_field_bytes(8, image.data.clone()),
    ])
}

/// Encode `agent.v1.ConversationHistoryImageContent { data 1: string, mime_type 2: string }`.
/// History images carry the data as a base64 string (not raw bytes).
fn encode_history_image_content(image: &CursorImage) -> Vec<u8> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&image.data);
    proto_message(vec![
        proto_field_string(1, &encoded),
        proto_field_string(2, &image.mime_type),
    ])
}

/// Encode `agent.v1.ConversationHistory { messages 1: repeated ConversationHistoryMessage }`.
fn encode_conversation_history(history: &[CursorHistoryMessage]) -> Vec<u8> {
    // Group a leading assistant text with its following tool calls into a single
    // assistant message where natural; here we emit one ConversationHistoryMessage
    // per item, which Cursor accepts (assistant text and tool calls are separate
    // content entries but separate messages are tolerated).
    let mut messages = Vec::new();
    for item in history {
        messages.push(proto_field_bytes(1, encode_history_message(item)));
    }
    proto_message(messages)
}

// `agent.v1.ConversationHistoryTextContent { text 1 }`
fn history_text_content(text: &str) -> Vec<u8> {
    proto_message(vec![proto_field_string(1, text)])
}

fn encode_history_message(item: &CursorHistoryMessage) -> Vec<u8> {
    match item {
        // ConversationHistoryMessage { user 1: ConversationHistoryUserMessage }
        // ConversationHistoryUserMessage { content 1: [ConversationHistoryUserContent] }
        // ConversationHistoryUserContent oneof: { text 1, image 2 }
        CursorHistoryMessage::User { text, images } => {
            let mut content = Vec::new();
            if !text.is_empty() || images.is_empty() {
                // ConversationHistoryUserContent { text 1: ConversationHistoryTextContent }
                content.push(proto_field_bytes(
                    1,
                    proto_message(vec![proto_field_bytes(1, history_text_content(text))]),
                ));
            }
            for image in images {
                // ConversationHistoryUserContent { image 2: ConversationHistoryImageContent }
                content.push(proto_field_bytes(2, encode_history_image_content(image)));
            }
            proto_message(vec![proto_field_bytes(1, proto_message(content))])
        }
        // ConversationHistoryMessage { assistant 2: ConversationHistoryAssistantMessage }
        // assistant content { text 1 }
        CursorHistoryMessage::AssistantText(text) => proto_message(vec![proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_bytes(1, history_text_content(text))]),
            )]),
        )]),
        // assistant content { tool_call 4: ConversationHistoryToolCall{ id 1, name 2, args_json 3 } }
        CursorHistoryMessage::AssistantToolCall {
            id,
            name,
            args_json,
        } => proto_message(vec![proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_bytes(
                    4,
                    proto_message(vec![
                        proto_field_string(1, id),
                        proto_field_string(2, name),
                        proto_field_string(3, args_json),
                    ]),
                )]),
            )]),
        )]),
        // ConversationHistoryMessage { tool 3: ConversationHistoryToolMessage }
        // { tool_call_id 1, tool_name 2, content 3: [ToolResultContent{ text 1 }], is_error 4 }
        CursorHistoryMessage::ToolResult {
            id,
            name,
            content,
            is_error,
        } => {
            let mut tool_message = vec![
                proto_field_string(1, id),
                proto_field_string(2, name),
                proto_field_bytes(
                    3,
                    proto_message(vec![proto_field_bytes(1, history_text_content(content))]),
                ),
            ];
            if *is_error {
                tool_message.push(proto_field_varint(4, 1));
            }
            proto_message(vec![proto_field_bytes(3, proto_message(tool_message))])
        }
    }
}

fn encode_requested_model(model_id: &str) -> Vec<u8> {
    // requested_model.f3 is a repeated {key, value} param list. cursor-agent
    // sends thinking/context/effort/fast; effort=high in particular drives the
    // model's full agentic thoroughness (without it the model does minimal work
    // and stops after a single read).
    let param = |key: &str, value: &str| {
        proto_field_bytes(
            3,
            proto_message(vec![
                proto_field_string(1, key),
                proto_field_string(2, value),
            ]),
        )
    };
    proto_message(vec![
        proto_field_string(1, model_id),
        param("thinking", "true"),
        param("context", "300k"),
        param("effort", "high"),
        param("fast", "false"),
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

/// Encode a 64-bit double field (wire type 1, fixed64 little-endian). Used for
/// `google.protobuf.Value.number_value`.
pub(crate) fn proto_field_double(no: u32, value: f64) -> Vec<u8> {
    let mut out = encode_varint(((no << 3) | 1) as u64);
    out.extend_from_slice(&value.to_bits().to_le_bytes());
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

// Cursor `agent.v1.ClientSideToolV2` `tool` oneof field numbers. Sourced from
// the Cursor app bundle's compiled protobuf schema
// (`agent-cli-worker` / `agent.v1`), not guessed. Each tool message is
// `{ 1: args, 2: result }`; the inner `*Args` field numbers are encoded in the
// mapping functions below.
const TOOL_SHELL: u32 = 1;
const TOOL_GLOB: u32 = 4;
const TOOL_GREP: u32 = 5;
const TOOL_READ: u32 = 8;
const TOOL_EDIT: u32 = 12;

fn decode_cursor_tool_call(bytes: &[u8]) -> Option<CursorToolCall> {
    let fields = decode_fields_safe(bytes);
    // Cursor tool-call ids are Anthropic-style `toolu_...` on the live
    // AgentService path (older traces used `tool_...`). Accept both.
    let id = fields
        .iter()
        .find_map(|field| bytes_field_as_string(field, 1))
        .filter(|value| value.starts_with("tool"))?;
    let payload = fields
        .iter()
        .find_map(|field| bytes_field(field, 2))
        .cloned()
        .unwrap_or_default();

    if let Some((name, arguments)) = map_cursor_native_tool(&payload) {
        return Some(CursorToolCall {
            id,
            name: name.to_string(),
            arguments: arguments.to_string(),
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

/// Map a decoded Cursor `ClientSideToolV2` payload to a canonical Roder tool
/// call (name + arguments JSON). Returns `None` when the Cursor-native tool has
/// no Roder equivalent yet, so the caller can surface `cursor_unsupported_tool`.
fn map_cursor_native_tool(payload: &[u8]) -> Option<(&'static str, Value)> {
    // read -> read_file { path, offset?, limit? }
    if let Some(args) = tool_args(payload, TOOL_READ) {
        let path = scalar_string(&args, 1).or_else(|| path_candidate_from_strings(payload))?;
        let mut obj = serde_json::Map::new();
        obj.insert("path".to_string(), json!(path));
        if let Some(offset) = scalar_u64(&args, 2) {
            obj.insert("offset".to_string(), json!(offset));
        }
        if let Some(limit) = scalar_u64(&args, 3) {
            obj.insert("limit".to_string(), json!(limit));
        }
        return Some(("read_file", Value::Object(obj)));
    }

    // edit -> write_file { path, content }. Cursor's edit streams the full new
    // file content in `stream_content` (EditArgs field 6), which matches
    // Roder's full-file write semantics rather than the old/new replace `edit`.
    if let Some(args) = tool_args(payload, TOOL_EDIT) {
        let path = scalar_string(&args, 1)?;
        let content = scalar_string(&args, 6).unwrap_or_default();
        return Some(("write_file", json!({ "path": path, "content": content })));
    }

    // shell -> shell { command, workdir? }
    if let Some(args) = tool_args(payload, TOOL_SHELL) {
        let command = scalar_string(&args, 1)?;
        let mut obj = serde_json::Map::new();
        obj.insert("command".to_string(), json!(command));
        if let Some(workdir) = scalar_string(&args, 2).filter(|value| !value.is_empty()) {
            obj.insert("workdir".to_string(), json!(workdir));
        }
        return Some(("shell", Value::Object(obj)));
    }

    // grep -> grep { query, path? }
    if let Some(args) = tool_args(payload, TOOL_GREP) {
        let query = scalar_string(&args, 1)?;
        let mut obj = serde_json::Map::new();
        obj.insert("query".to_string(), json!(query));
        if let Some(path) = scalar_string(&args, 2).filter(|value| !value.is_empty()) {
            obj.insert("path".to_string(), json!(path));
        }
        return Some(("grep", Value::Object(obj)));
    }

    // glob -> glob { pattern } (Cursor GlobToolArgs.glob_pattern is field 2)
    if let Some(args) = tool_args(payload, TOOL_GLOB) {
        let pattern = scalar_string(&args, 2)?;
        return Some(("glob", json!({ "pattern": pattern })));
    }

    // Legacy resilience: heuristic read-path detection if the structured args
    // were not present in the expected shape.
    if let Some(path) =
        nested_string(payload, &[TOOL_READ, 1, 1]).or_else(|| path_candidate_from_strings(payload))
    {
        return Some(("read_file", json!({ "path": path })));
    }

    None
}

// ===== Exec channel (agent.v1 AgentService bidi runtime) =====
//
// AgentServerMessage oneof: 1 = interaction_update, 2 = exec_server_message.
// AgentClientMessage oneof: 1 = run_request, 2 = exec_client_message,
//   5 = exec_client_control_message, 7 = client_heartbeat.
// ExecServerMessage: f1 = seq; oneof request f7=READ, f3=WRITE, f14=SHELL, f10=INIT.
// ExecClientMessage: f1 = seq; oneof result mirrors the request field number.

/// A tool-execution request the server asks the client to run mid-stream.
#[derive(Debug, Clone)]
pub(crate) enum CursorExecRequest {
    Read {
        seq: u64,
        path: String,
        tool_call_id: String,
    },
    Write {
        seq: u64,
        path: String,
        content: Vec<u8>,
        tool_call_id: String,
    },
    Shell {
        seq: u64,
        command: String,
        cwd: String,
        tool_call_id: String,
    },
    /// Unified ripgrep search (exec field 5): glob (`files_with_matches`) or
    /// grep (`content`).
    Search {
        seq: u64,
        pattern: Option<String>,
        path: String,
        glob: Option<String>,
        mode: String,
        tool_call_id: String,
    },
    /// The server requests workspace context (`request_context_args`, field 10
    /// of ExecServerMessage) before it will start generating text. The client
    /// must respond with `encode_exec_request_context_result` to unblock the
    /// model.  `id` mirrors ExecServerMessage.f1 and must be echoed back.
    RequestContext { id: u64 },
}

/// One grep match: a relative file path, a 1-based line number, and the line.
#[derive(Debug, Clone)]
pub(crate) struct CursorGrepMatch {
    pub path: String,
    pub line: u64,
    pub text: String,
}

/// Decoded `AgentServerMessage` for the bidi runtime: model text/thinking,
/// turn-end signal, and any exec request to service.
#[derive(Debug, Default, Clone)]
pub(crate) struct CursorServerFrame {
    pub text: String,
    pub thinking: String,
    pub turn_ended: bool,
    pub exec: Option<CursorExecRequest>,
    /// Sequence of a `kv_server` PUT that must be acked with [`encode_kv_ack`].
    pub kv_seq: Option<u64>,
}

pub(crate) fn decode_server_frame(payload: &[u8]) -> CursorServerFrame {
    let interaction = decode_agent_server_message(payload);
    let mut frame = CursorServerFrame {
        text: interaction.text,
        thinking: interaction.thinking,
        turn_ended: interaction.turn_ended,
        exec: None,
        kv_seq: None,
    };
    // field 2 = exec_server_message
    if let Some(exec_bytes) = submessage(payload, 2) {
        frame.exec = decode_exec_server(&exec_bytes);
    }
    // field 4 = kv_server_message (PUT of conversation state); ack by seq.
    if let Some(kv_bytes) = submessage(payload, 4) {
        frame.kv_seq = Some(scalar_u64(&kv_bytes, 1).unwrap_or(0));
    }
    frame
}

/// kv_client ack: `AgentClientMessage{ 3:{ 1:seq, 3:<empty> } }`.
pub(crate) fn encode_kv_ack(seq: u64) -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        3,
        proto_message(vec![
            proto_field_varint(1, seq),
            proto_field_bytes(3, Vec::new()),
        ]),
    )])
}

fn decode_exec_server(bytes: &[u8]) -> Option<CursorExecRequest> {
    let seq = scalar_u64(bytes, 1).unwrap_or(0);
    if let Some(read) = submessage(bytes, 7) {
        let path = scalar_string(&read, 1)?;
        let tool_call_id = scalar_string(&read, 2).unwrap_or_default();
        return Some(CursorExecRequest::Read {
            seq,
            path,
            tool_call_id,
        });
    }
    if let Some(write) = submessage(bytes, 3) {
        let path = scalar_string(&write, 1)?;
        let content = submessage(&write, 2).unwrap_or_default();
        let tool_call_id = scalar_string(&write, 3).unwrap_or_default();
        return Some(CursorExecRequest::Write {
            seq,
            path,
            content,
            tool_call_id,
        });
    }
    if let Some(shell) = submessage(bytes, 14) {
        let command = scalar_string(&shell, 1)?;
        let cwd = scalar_string(&shell, 2).unwrap_or_default();
        let tool_call_id = scalar_string(&shell, 4).unwrap_or_default();
        return Some(CursorExecRequest::Shell {
            seq,
            command,
            cwd,
            tool_call_id,
        });
    }
    if let Some(search) = submessage(bytes, 5) {
        let pattern = scalar_string(&search, 1);
        let path = scalar_string(&search, 2).unwrap_or_default();
        let glob = scalar_string(&search, 3);
        let mode = scalar_string(&search, 4).unwrap_or_default();
        let tool_call_id = scalar_string(&search, 14).unwrap_or_default();
        return Some(CursorExecRequest::Search {
            seq,
            pattern,
            path,
            glob,
            mode,
            tool_call_id,
        });
    }
    if submessage(bytes, 10).is_some() {
        return Some(CursorExecRequest::RequestContext { id: seq });
    }
    None
}

fn wrap_exec_client(exec: Vec<u8>) -> Vec<u8> {
    // AgentClientMessage { 2: exec_client_message }
    proto_message(vec![proto_field_bytes(2, exec)])
}

/// READ result: ExecClientMessage{ 1:seq, 7:{ 1:{ 1:path, 2:content, 3:total_lines, 4:file_size } } }
pub(crate) fn encode_exec_read_result(
    seq: u64,
    path: &str,
    content: &[u8],
    total_lines: u64,
) -> Vec<u8> {
    let inner = proto_message(vec![
        proto_field_string(1, path),
        proto_field_bytes(2, content.to_vec()),
        proto_field_varint(3, total_lines),
        proto_field_varint(4, content.len() as u64),
    ]);
    let read = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(7, read),
    ]))
}

/// WRITE result: ExecClientMessage{ 1:seq, 3:{ 1:{ 1:path, 2:lines, 3:size } } }
pub(crate) fn encode_exec_write_result(seq: u64, path: &str, lines: u64, size: u64) -> Vec<u8> {
    let inner = proto_message(vec![
        proto_field_string(1, path),
        proto_field_varint(2, lines),
        proto_field_varint(3, size),
    ]);
    let write = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(3, write),
    ]))
}

/// SHELL result, streamed as three ExecClientMessages with the same seq:
///   start:  14:{ 4:{ 1:{ 1:1 } } }
///   stdout: 14:{ 1:{ 1:{ 7:stdout } } }
///   exit:   14:{ 3:{ 2:cwd, 6:byte_count } }
pub(crate) fn encode_exec_shell_results(seq: u64, cwd: &str, stdout: &str) -> Vec<Vec<u8>> {
    let shell_msg = |inner: Vec<u8>| {
        wrap_exec_client(proto_message(vec![
            proto_field_varint(1, seq),
            proto_field_bytes(14, inner),
        ]))
    };
    let start = shell_msg(proto_message(vec![proto_field_bytes(
        4,
        proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_varint(1, 1)]),
        )]),
    )]));
    // stdout: 14:{ 1:{ 1:stdout } }
    let out = shell_msg(proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_string(1, stdout)]),
    )]));
    // exit: 14:{ 3:{ 2:cwd, 6:byte_count } }
    let exit = shell_msg(proto_message(vec![proto_field_bytes(
        3,
        proto_message(vec![
            proto_field_string(2, cwd),
            proto_field_varint(6, stdout.len() as u64),
        ]),
    )]));
    vec![start, out, exit]
}

/// Search result, `files_with_matches` (glob) mode:
/// `2:{ 5:{ 1:{ 2:path, 3:"files_with_matches", 4:{ 1:root, 2:{ 2:{ 1:relpath*, 2:count } } } } } }`
pub(crate) fn encode_exec_glob_result(
    seq: u64,
    path: &str,
    root: &str,
    rel_paths: &[String],
) -> Vec<u8> {
    let mut files: Vec<Vec<u8>> = rel_paths.iter().map(|p| proto_field_string(1, p)).collect();
    files.push(proto_field_varint(2, rel_paths.len() as u64));
    let f4 = proto_message(vec![
        proto_field_string(1, root),
        proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(2, proto_message(files))]),
        ),
    ]);
    let inner = proto_message(vec![
        proto_field_string(2, path),
        proto_field_string(3, "files_with_matches"),
        proto_field_bytes(4, f4),
    ]);
    let search = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(5, search),
    ]))
}

/// Search result, `content` (grep) mode:
/// `2:{ 5:{ 1:{ 1:pattern, 2:path, 3:"content", 4:{ 1:root, 2:{ 3:{ 1:{1:relpath,2:{1:line,2:text}}*, 2:count, 3:count } } } } } }`
pub(crate) fn encode_exec_grep_result(
    seq: u64,
    pattern: &str,
    path: &str,
    root: &str,
    matches: &[CursorGrepMatch],
) -> Vec<u8> {
    let mut entries: Vec<Vec<u8>> = matches
        .iter()
        .map(|m| {
            proto_field_bytes(
                1,
                proto_message(vec![
                    proto_field_string(1, &m.path),
                    proto_field_bytes(
                        2,
                        proto_message(vec![
                            proto_field_varint(1, m.line),
                            proto_field_string(2, &m.text),
                        ]),
                    ),
                ]),
            )
        })
        .collect();
    entries.push(proto_field_varint(2, matches.len() as u64));
    entries.push(proto_field_varint(3, matches.len() as u64));
    let f4 = proto_message(vec![
        proto_field_string(1, root),
        proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(3, proto_message(entries))]),
        ),
    ]);
    let inner = proto_message(vec![
        proto_field_string(1, pattern),
        proto_field_string(2, path),
        proto_field_string(3, "content"),
        proto_field_bytes(4, f4),
    ]);
    let search = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(5, search),
    ]))
}

/// REQUEST-CONTEXT result: responds to `request_context_args` (field 10 of
/// `ExecServerMessage`).  The server requires this before it will start
/// generating model output.
///
/// Protobuf:
///   `AgentClientMessage{ 2: ExecClientMessage{
///       1: id,
///       10: RequestContextResult{
///           1: RequestContextSuccess{
///               1: RequestContext{
///                   4: RequestContextEnv{
///                       1: os_version,
///                       2: workspace_paths (repeated),
///                       11: project_folder } } } } } }`
pub(crate) fn encode_exec_request_context_result(id: u64, workspace_path: &str) -> Vec<u8> {
    let env = proto_message(vec![
        proto_field_string(1, "macOS"),
        proto_field_string(2, workspace_path),   // workspace_paths[]
        proto_field_string(11, workspace_path),  // project_folder
    ]);
    let context = proto_message(vec![proto_field_bytes(4, env)]);
    let success = proto_message(vec![proto_field_bytes(1, context)]);
    let result = proto_message(vec![proto_field_bytes(1, success)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, id),
        proto_field_bytes(10, result),
    ]))
}

/// exec_client INIT (context handshake): `AgentClientMessage{ 2:{ 10:{ 1:{ 1:{ 2:[files] } } } } }`.
/// Each file entry is `{ 1: path, 2: content }`. An empty list establishes the
/// exec channel without pushing workspace files (the model reads on demand).
///
/// Kept as a documented encoder for the exec INIT frame (see
/// docs/roder-cursor-agent-runtime-protocol.md): the live path responds to the
/// server's `request_context_args` instead of proactively pushing files, so
/// this is currently exercised only by unit tests.
#[allow(dead_code)]
pub(crate) fn encode_exec_init(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<Vec<u8>> = files
        .iter()
        .map(|(path, content)| {
            proto_field_bytes(
                2,
                proto_message(vec![
                    proto_field_string(1, path),
                    proto_field_bytes(2, content.clone()),
                ]),
            )
        })
        .collect();
    let file_list = proto_message(entries);
    let ctx = proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_bytes(1, file_list)]),
    )]);
    wrap_exec_client(proto_message(vec![proto_field_bytes(10, ctx)]))
}

/// client_heartbeat keepalive: AgentClientMessage{ 7:<empty> }. Long turns are
/// reset by the server without periodic heartbeats.
pub(crate) fn encode_heartbeat() -> Vec<u8> {
    proto_message(vec![proto_field_bytes(7, Vec::new())])
}

/// exec_client_control_message ack: AgentClientMessage{ 5:{ 1:<empty> } }
pub(crate) fn encode_exec_control() -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        5,
        proto_message(vec![proto_field_bytes(1, Vec::new())]),
    )])
}

/// First length-delimited (sub-message / string) field with the given number.
fn submessage(bytes: &[u8], no: u32) -> Option<Vec<u8>> {
    decode_fields_safe(bytes)
        .iter()
        .find_map(|field| bytes_field(field, no).cloned())
}

/// Extract a tool's `args` sub-message: `payload.<tool_no>.1` where
/// `<tool_no>` is the `*ToolCall` message and field `1` is its `*Args`.
fn tool_args(payload: &[u8], tool_no: u32) -> Option<Vec<u8>> {
    submessage(&submessage(payload, tool_no)?, 1)
}

/// Decode a scalar `string`/`bytes` field as UTF-8 (lossy). Unlike
/// [`bytes_field_as_string`] this does not apply the printable-text heuristic,
/// because file contents and commands are read from known scalar field numbers.
fn scalar_string(bytes: &[u8], no: u32) -> Option<String> {
    decode_fields_safe(bytes).iter().find_map(|field| {
        bytes_field(field, no).map(|value| String::from_utf8_lossy(value).into_owned())
    })
}

/// Decode a scalar varint field.
fn scalar_u64(bytes: &[u8], no: u32) -> Option<u64> {
    decode_fields_safe(bytes)
        .iter()
        .find_map(|field| match &field.value {
            ProtoValue::Varint(value) if field.no == no => Some(*value),
            _ => None,
        })
}

fn bytes_field(field: &ProtoField, no: u32) -> Option<&Vec<u8>> {
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
    let (first, rest) = path.split_first()?;
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
#[path = "proto_exec_tests.rs"]
mod proto_exec_tests;

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
    fn exec_init_encodes_workspace_files_into_handshake() {
        // AgentClientMessage{ 2:{ 10:{ 1:{ 1:{ 2:[ {1:path, 2:content} ] } } } } }.
        let bytes = encode_exec_init(&[("AGENTS.md".to_string(), b"file body".to_vec())]);
        let exec = submessage(&bytes, 2).expect("exec_client_message");
        let ctx = submessage(&exec, 10).expect("context (field 10)");
        let l1 = submessage(&ctx, 1).expect("context.1");
        let file_list = submessage(&l1, 1).expect("file_list");
        let file = submessage(&file_list, 2).expect("first file entry");
        assert_eq!(scalar_string(&file, 1).as_deref(), Some("AGENTS.md"));
        assert_eq!(submessage(&file, 2).as_deref(), Some(b"file body".as_slice()));

        // An empty file list still establishes the exec channel (no file entries).
        let empty = encode_exec_init(&[]);
        let exec = submessage(&empty, 2).expect("exec_client_message");
        let ctx = submessage(&exec, 10).expect("context (field 10)");
        let l1 = submessage(&ctx, 1).expect("context.1");
        let file_list = submessage(&l1, 1).expect("file_list");
        assert!(submessage(&file_list, 2).is_none());
    }

    #[test]
    fn agent_client_message_contains_prompt_and_model() {
        let bytes = encode_agent_client_message_with_history(
            "hello cursor",
            "composer-2.5",
            "conv",
            "msg",
            &[],
            &[],
            &[],
        );
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

    /// Build a server message carrying one Cursor `ClientSideToolV2` tool call,
    /// matching the on-wire shape: interaction-update(1) -> tool-call(2) ->
    /// { id(1), ClientSideToolV2(2) }.
    fn tool_update(id: &str, client_side_tool_v2: Vec<u8>) -> Vec<u8> {
        proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_bytes(
                2,
                proto_message(vec![
                    proto_field_string(1, id),
                    proto_field_bytes(2, client_side_tool_v2),
                    proto_field_string(3, "model-call-0"),
                ]),
            )]),
        )])
    }

    /// `<tool_no>: { 1: { <args> } }` — the `*ToolCall { args, result }` wrapper.
    fn tool_with_args(tool_no: u32, args: Vec<u8>) -> Vec<u8> {
        proto_message(vec![proto_field_bytes(
            tool_no,
            proto_message(vec![proto_field_bytes(1, args)]),
        )])
    }

    fn decode_one(payload: &[u8]) -> CursorToolCall {
        let decoded = decode_agent_server_message(payload);
        assert_eq!(
            decoded.tool_calls.len(),
            1,
            "expected exactly one tool call"
        );
        decoded.tool_calls.into_iter().next().unwrap()
    }

    #[test]
    fn user_message_requests_agent_mode_not_ask() {
        // Regression: roder must send UserMessage.mode = AGENT_MODE_AGENT (1).
        // Sending 2 (AGENT_MODE_ASK) made Cursor run the model read-only.
        let bytes =
            encode_agent_client_message_with_history("hi", "claude-opus-4-8", "conv", "msg", &[], &[], &[]);
        let run = submessage(&bytes, 1).expect("agent run request");
        let action = submessage(&run, 2).expect("conversation action");
        let user_message_action = submessage(&action, 1).expect("user message action");
        let user_message = submessage(&user_message_action, 1).expect("user message");
        assert_eq!(scalar_u64(&user_message, 4), Some(AGENT_MODE_AGENT));
    }

    #[test]
    fn decodes_tool_call_with_anthropic_style_toolu_id() {
        // Regression: live Cursor tool ids are `toolu_...`; the decoder must not
        // reject them (the old `tool_` prefix filter dropped every real call).
        let args = proto_message(vec![proto_field_string(1, "AGENTS.md")]);
        let call = decode_one(&tool_update(
            "toolu_015B6aNmUMzPiezhHL6Zbtey",
            tool_with_args(TOOL_READ, args),
        ));
        assert_eq!(call.id, "toolu_015B6aNmUMzPiezhHL6Zbtey");
        assert_eq!(call.name, "read_file");
        let value: Value = serde_json::from_str(&call.arguments).unwrap();
        assert_eq!(value["path"], "AGENTS.md");
    }

    #[test]
    fn conversation_history_encodes_user_assistant_toolcall_and_result() {
        let history = vec![
            CursorHistoryMessage::AssistantToolCall {
                id: "toolu_1".to_string(),
                name: "read_file".to_string(),
                args_json: r#"{"path":"AGENTS.md"}"#.to_string(),
            },
            CursorHistoryMessage::ToolResult {
                id: "toolu_1".to_string(),
                name: "read_file".to_string(),
                content: "file body".to_string(),
                is_error: false,
            },
        ];
        let bytes = encode_agent_client_message_with_history("edit it", "m", "c", "mid", &history, &[], &[]);
        // ConversationHistory lives at AgentRunRequest(1).action(2).user_message_action(1).conversation_history(7).
        let run = submessage(&bytes, 1).unwrap();
        let action = submessage(&run, 2).unwrap();
        let uma = submessage(&action, 1).unwrap();
        let conv_history = submessage(&uma, 7).expect("conversation_history present");
        // messages(1) repeated; assert the encoded bytes carry the tool-call id, args, and result text.
        let text = String::from_utf8_lossy(&conv_history);
        assert!(text.contains("toolu_1"));
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("file body"));
    }

    #[test]
    fn advertises_roder_tools_as_mcp_tool_definitions_with_value_schema() {
        let tool = CursorMcpTool {
            name: "grep".to_string(),
            description: "Search text".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        };
        let bytes = encode_agent_client_message_with_history(
            "find foo",
            "claude-opus-4-8",
            "conv",
            "msg",
            &[],
            &[],
            std::slice::from_ref(&tool),
        );
        // AgentRunRequest.mcp_tools(4).mcp_tools(1) = McpToolDefinition.
        let run = submessage(&bytes, 1).unwrap();
        let mcp_tools = submessage(&run, 4).expect("mcp_tools present");
        let def = submessage(&mcp_tools, 1).expect("tool definition present");
        assert_eq!(scalar_string(&def, 1).as_deref(), Some("grep"));
        assert_eq!(scalar_string(&def, 2).as_deref(), Some("Search text"));
        // input_schema(3) is a google.protobuf.Value carrying a struct_value(5).
        let input_schema = submessage(&def, 3).expect("input_schema present");
        let struct_value = submessage(&input_schema, 5).expect("struct_value present");
        // The "type":"object" entry: map entry { key 1: "type", value 2: Value }
        // whose Value.string_value (field 3) is "object".
        let type_string = decode_fields_safe(&struct_value)
            .iter()
            .filter_map(|field| bytes_field(field, 1).cloned())
            .find(|entry| scalar_string(entry, 1).as_deref() == Some("type"))
            .and_then(|entry| submessage(&entry, 2))
            .and_then(|value| scalar_string(&value, 3));
        assert_eq!(type_string.as_deref(), Some("object"));
    }

    #[test]
    fn omitting_tools_keeps_mcp_tools_message_empty() {
        let bytes = encode_agent_client_message_with_history(
            "hi", "m", "c", "mid", &[], &[], &[],
        );
        let run = submessage(&bytes, 1).unwrap();
        // mcp_tools(4) is present but its repeated definition list is empty.
        let mcp_tools = submessage(&run, 4).unwrap_or_default();
        assert!(submessage(&mcp_tools, 1).is_none());
    }

    #[test]
    fn parses_base64_data_url_into_bytes_and_mime() {
        let image = CursorImage::from_data_url("data:image/png;base64,UE5HREFUQQ==").unwrap();
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.data, b"PNGDATA");
        // Remote URLs and non-base64 data URLs cannot carry inline bytes.
        assert!(CursorImage::from_data_url("https://example.com/x.png").is_none());
        assert!(CursorImage::from_data_url("data:image/png,raw").is_none());
    }

    #[test]
    fn current_message_image_rides_in_selected_context_and_sets_inline_flag() {
        let image = CursorImage {
            mime_type: "image/png".to_string(),
            data: b"PNGDATA".to_vec(),
        };
        let bytes = encode_agent_client_message_with_history(
            "look at this",
            "claude-opus-4-8",
            "conv",
            "msg",
            &[],
            std::slice::from_ref(&image),
            &[],
        );
        let run = submessage(&bytes, 1).unwrap();
        // AgentRunRequest.client_supports_inline_images (field 19) must be set.
        assert_eq!(scalar_u64(&run, 19), Some(1));
        let action = submessage(&run, 2).unwrap();
        let uma = submessage(&action, 1).unwrap();
        let user_message = submessage(&uma, 1).unwrap();
        // UserMessage.selected_context(3).selected_images(1) = SelectedImage.
        let selected_context = submessage(&user_message, 3).expect("selected_context present");
        let selected_image = submessage(&selected_context, 1).expect("selected_image present");
        // SelectedImage { mime_type 7, data 8 }.
        assert_eq!(scalar_string(&selected_image, 7).as_deref(), Some("image/png"));
        assert_eq!(scalar_string(&selected_image, 8).as_deref(), Some("PNGDATA"));
    }

    #[test]
    fn turn_without_images_keeps_empty_selected_context_and_no_inline_flag() {
        let bytes =
            encode_agent_client_message_with_history("hello", "m", "c", "mid", &[], &[], &[]);
        let run = submessage(&bytes, 1).unwrap();
        assert_eq!(scalar_u64(&run, 19), None);
        let action = submessage(&run, 2).unwrap();
        let uma = submessage(&action, 1).unwrap();
        let user_message = submessage(&uma, 1).unwrap();
        // selected_context (field 3) is present but empty (zero-length message).
        assert_eq!(submessage(&user_message, 3), Some(Vec::new()));
    }

    #[test]
    fn history_user_image_encodes_as_image_content() {
        let history = vec![CursorHistoryMessage::User {
            text: "earlier".to_string(),
            images: vec![CursorImage {
                mime_type: "image/jpeg".to_string(),
                data: b"JPEGDATA".to_vec(),
            }],
        }];
        use base64::Engine as _;
        let bytes = encode_agent_client_message_with_history("now", "m", "c", "mid", &history, &[], &[]);
        let run = submessage(&bytes, 1).unwrap();
        // History images also require the inline-images capability flag.
        assert_eq!(scalar_u64(&run, 19), Some(1));
        let action = submessage(&run, 2).unwrap();
        let uma = submessage(&action, 1).unwrap();
        let conv_history = submessage(&uma, 7).expect("conversation_history present");
        let message = submessage(&conv_history, 1).unwrap();
        let user = submessage(&message, 1).unwrap();
        // ConversationHistoryUserContent.image(2) -> ImageContent { data 1, mime_type 2 }.
        let image_content = submessage(&user, 2).expect("image content present");
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"JPEGDATA");
        assert_eq!(scalar_string(&image_content, 1).as_deref(), Some(encoded.as_str()));
        assert_eq!(
            scalar_string(&image_content, 2).as_deref(),
            Some("image/jpeg")
        );
    }

    #[test]
    fn maps_cursor_edit_tool_call_to_write_file() {
        // EditToolCall(12) -> EditArgs { path(1), stream_content(6) }
        let args = proto_message(vec![
            proto_field_string(1, "src/lib.rs"),
            proto_field_string(6, "fn added() {}\n"),
        ]);
        let call = decode_one(&tool_update("tool_edit_1", tool_with_args(TOOL_EDIT, args)));
        assert_eq!(call.name, "write_file");
        let value: Value = serde_json::from_str(&call.arguments).unwrap();
        assert_eq!(value["path"], "src/lib.rs");
        assert_eq!(value["content"], "fn added() {}\n");
    }

    #[test]
    fn maps_cursor_shell_tool_call_to_shell() {
        // ShellToolCall(1) -> ShellArgs { command(1), working_directory(2) }
        let args = proto_message(vec![
            proto_field_string(1, "cargo test -p roder-ext-cursor"),
            proto_field_string(2, "/repo"),
        ]);
        let call = decode_one(&tool_update(
            "tool_shell_1",
            tool_with_args(TOOL_SHELL, args),
        ));
        assert_eq!(call.name, "shell");
        let value: Value = serde_json::from_str(&call.arguments).unwrap();
        assert_eq!(value["command"], "cargo test -p roder-ext-cursor");
        assert_eq!(value["workdir"], "/repo");
    }

    #[test]
    fn maps_cursor_read_tool_call_with_offset_and_limit() {
        // ReadToolCall(8) -> ReadToolArgs { path(1), offset(2), limit(3) }
        let args = proto_message(vec![
            proto_field_string(1, "README.md"),
            proto_field_varint(2, 10),
            proto_field_varint(3, 50),
        ]);
        let call = decode_one(&tool_update("tool_read_2", tool_with_args(TOOL_READ, args)));
        assert_eq!(call.name, "read_file");
        let value: Value = serde_json::from_str(&call.arguments).unwrap();
        assert_eq!(value["path"], "README.md");
        assert_eq!(value["offset"], 10);
        assert_eq!(value["limit"], 50);
    }

    #[test]
    fn maps_cursor_grep_and_glob_tool_calls() {
        // GrepToolCall(5) -> GrepArgs { pattern(1), path(2) }
        let grep_args = proto_message(vec![
            proto_field_string(1, "TODO"),
            proto_field_string(2, "crates"),
        ]);
        let grep = decode_one(&tool_update(
            "tool_grep_1",
            tool_with_args(TOOL_GREP, grep_args),
        ));
        assert_eq!(grep.name, "grep");
        let grep_value: Value = serde_json::from_str(&grep.arguments).unwrap();
        assert_eq!(grep_value["query"], "TODO");
        assert_eq!(grep_value["path"], "crates");

        // GlobToolCall(4) -> GlobToolArgs { glob_pattern(2) }
        let glob_args = proto_message(vec![proto_field_string(2, "**/*.rs")]);
        let glob = decode_one(&tool_update(
            "tool_glob_1",
            tool_with_args(TOOL_GLOB, glob_args),
        ));
        assert_eq!(glob.name, "glob");
        let glob_value: Value = serde_json::from_str(&glob.arguments).unwrap();
        assert_eq!(glob_value["pattern"], "**/*.rs");
    }
}
