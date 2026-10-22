use super::*;

// ===== Exec channel (agent.v1 AgentService bidi runtime) =====
//
// AgentServerMessage oneof: 1 = interaction_update, 2 = exec_server_message.
// AgentClientMessage oneof: 1 = run_request, 2 = exec_client_message,
//   5 = exec_client_control_message, 7 = client_heartbeat.
// ExecServerMessage: f1 = seq; oneof request f7=READ, f3=WRITE, f14=SHELL, f10=INIT.
// ExecClientMessage: f1 = seq; native result slots below match requests.
// Newer Pi request/result numbers differ; unknown slots use ExecClientThrow.

/// A tool-execution request the server asks the client to run mid-stream.
#[derive(Debug, Clone)]
pub(crate) enum CursorExecRequest {
    Read {
        seq: u64,
        path: String,
        tool_call_id: String,
        field_no: u32,
        offset: Option<u64>,
        limit: Option<u64>,
    },
    Write {
        seq: u64,
        path: String,
        content: Vec<u8>,
        tool_call_id: String,
    },
    /// Delete a file (exec field 4): `{ f1: path, f2: tool_call_id }`.
    /// Wire shape confirmed from a live composer-2.5 capture (the model's
    /// native file-deletion tool).
    Delete {
        seq: u64,
        path: String,
        tool_call_id: String,
    },
    Shell {
        seq: u64,
        command: String,
        cwd: String,
        tool_call_id: String,
        field_no: u32,
        timeout_ms: Option<u64>,
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
        case_insensitive: bool,
        multiline: bool,
        offset: u64,
        limit: Option<u64>,
    },
    List {
        seq: u64,
        path: String,
        tool_call_id: String,
    },
    Diagnostics {
        seq: u64,
        path: String,
        tool_call_id: String,
    },
    Mcp {
        seq: u64,
        name: String,
        tool_call_id: String,
        arguments: Value,
    },
    McpState {
        seq: u64,
        server_identifiers: Vec<String>,
    },
    /// The server requests workspace context (`request_context_args`, field 10
    /// of ExecServerMessage) before it will start generating text. The client
    /// must respond with `encode_exec_request_context_result` to unblock the
    /// model.  `id` mirrors ExecServerMessage.f1 and must be echoed back.
    RequestContext { id: u64 },
    /// An unimplemented exec request. Return ExecClientThrow with the id;
    /// never fabricate success or assume its result uses the same field number.
    Unknown { seq: u64, field_no: u32 },
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
    pub usage_fields: BTreeMap<u32, u64>,
    pub turn_ended: bool,
    pub exec: Option<CursorExecRequest>,
    /// Sequence of a `kv_server` PUT that must be acked with [`encode_kv_ack`].
    pub kv_seq: Option<u64>,
    pub kv_put: Option<(Vec<u8>, Vec<u8>)>,
    pub kv_get: Option<(u64, Vec<u8>)>,
    pub checkpoint: Option<Vec<u8>>,
}

pub(crate) fn decode_server_frame(payload: &[u8]) -> CursorServerFrame {
    let interaction = decode_agent_server_message(payload);
    let mut frame = CursorServerFrame {
        text: interaction.text,
        thinking: interaction.thinking,
        usage_fields: interaction.usage_fields,
        turn_ended: interaction.turn_ended,
        exec: None,
        kv_seq: None,
        kv_put: None,
        kv_get: None,
        checkpoint: submessage(payload, 3),
    };
    // field 2 = exec_server_message
    if let Some(exec_bytes) = submessage(payload, 2) {
        frame.exec = decode_exec_server(&exec_bytes);
    }
    // field 4 = kv_server_message (PUT of conversation state); ack by seq.
    if let Some(kv_bytes) = submessage(payload, 4) {
        let seq = scalar_u64(&kv_bytes, 1).unwrap_or(0);
        if let Some(get) = submessage(&kv_bytes, 2) {
            frame.kv_get = Some((seq, submessage(&get, 1).unwrap_or_default()));
        } else if let Some(put) = submessage(&kv_bytes, 3) {
            frame.kv_seq = Some(seq);
            frame.kv_put = Some((
                submessage(&put, 1).unwrap_or_default(),
                submessage(&put, 2).unwrap_or_default(),
            ));
        }
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

/// `ExecServerMessage` length-delimited fields that are envelope metadata, not
/// the request oneof: f15 = message uuid, f19 = routing keys (f1 = seq is a
/// varint and never matches the length-delimited scan).
const EXEC_METADATA_FIELDS: [u32; 4] = [15, 19, 55, 57];

fn decode_exec_server(bytes: &[u8]) -> Option<CursorExecRequest> {
    let seq = scalar_u64(bytes, 1).unwrap_or(0);
    for field_no in [7, 29] {
        if let Some(read) = submessage(bytes, field_no) {
            if let Some(path) = scalar_string(&read, 1) {
                let tool_call_id = scalar_string(&read, 2).unwrap_or_default();
                return Some(CursorExecRequest::Read {
                    seq,
                    path,
                    tool_call_id,
                    field_no,
                    offset: scalar_u64(&read, 4),
                    limit: scalar_u64(&read, 5),
                });
            }
            return Some(CursorExecRequest::Unknown { seq, field_no });
        }
    }
    if let Some(write) = submessage(bytes, 3) {
        if let Some(path) = scalar_string(&write, 1) {
            let content = submessage(&write, 5)
                .or_else(|| submessage(&write, 2))
                .unwrap_or_default();
            let tool_call_id = scalar_string(&write, 3).unwrap_or_default();
            return Some(CursorExecRequest::Write {
                seq,
                path,
                content,
                tool_call_id,
            });
        }
        return Some(CursorExecRequest::Unknown { seq, field_no: 3 });
    }
    if let Some(delete) = submessage(bytes, 4) {
        if let Some(path) = scalar_string(&delete, 1) {
            let tool_call_id = scalar_string(&delete, 2).unwrap_or_default();
            return Some(CursorExecRequest::Delete {
                seq,
                path,
                tool_call_id,
            });
        }
        return Some(CursorExecRequest::Unknown { seq, field_no: 4 });
    }
    for field_no in [2, 14] {
        if let Some(shell) = submessage(bytes, field_no) {
            if let Some(command) = scalar_string(&shell, 1) {
                let cwd = scalar_string(&shell, 2).unwrap_or_default();
                let tool_call_id = scalar_string(&shell, 4).unwrap_or_default();
                return Some(CursorExecRequest::Shell {
                    seq,
                    command,
                    cwd,
                    tool_call_id,
                    field_no,
                    timeout_ms: scalar_u64(&shell, 3),
                });
            }
            return Some(CursorExecRequest::Unknown { seq, field_no });
        }
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
            case_insensitive: scalar_u64(&search, 8).unwrap_or(0) != 0,
            multiline: scalar_u64(&search, 11).unwrap_or(0) != 0,
            limit: scalar_u64(&search, 10),
            offset: scalar_u64(&search, 16).unwrap_or(0),
        });
    }
    for field_no in [8, 9] {
        if let Some(args) = submessage(bytes, field_no) {
            let path = scalar_string(&args, 1).unwrap_or_else(|| ".".to_string());
            let tool_call_id =
                scalar_string(&args, if field_no == 8 { 3 } else { 2 }).unwrap_or_default();
            return Some(if field_no == 8 {
                CursorExecRequest::List {
                    seq,
                    path,
                    tool_call_id,
                }
            } else {
                CursorExecRequest::Diagnostics {
                    seq,
                    path,
                    tool_call_id,
                }
            });
        }
    }
    if let Some(args) = submessage(bytes, 11) {
        let name = scalar_string(&args, 5)
            .filter(|name| !name.is_empty())
            .or_else(|| scalar_string(&args, 1))
            .unwrap_or_default();
        let mut arguments = serde_json::Map::new();
        for field in decode_fields_safe(&args)
            .into_iter()
            .filter(|field| field.no == 2)
        {
            if let ProtoValue::Bytes(entry) = field.value
                && let (Some(key), Some(value)) = (scalar_string(&entry, 1), submessage(&entry, 2))
            {
                arguments.insert(key, decode_protobuf_value(&value));
            }
        }
        return Some(CursorExecRequest::Mcp {
            seq,
            name,
            tool_call_id: scalar_string(&args, 3).unwrap_or_default(),
            arguments: Value::Object(arguments),
        });
    }
    if let Some(args) = submessage(bytes, 36) {
        let server_identifiers = decode_fields_safe(&args)
            .into_iter()
            .filter_map(|field| match (field.no, field.value) {
                (1, ProtoValue::Bytes(value)) => String::from_utf8(value).ok(),
                _ => None,
            })
            .collect();
        return Some(CursorExecRequest::McpState {
            seq,
            server_identifiers,
        });
    }
    if submessage(bytes, 10).is_some() {
        return Some(CursorExecRequest::RequestContext { id: seq });
    }
    // Any other request slot in the oneof (delete, ls, todo, MCP state, ...)
    // that Roder does not implement yet. Still surface it so the caller can
    // reply and keep the stream moving.
    decode_fields_safe(bytes).into_iter().find_map(|field| {
        let ProtoValue::Bytes(_) = field.value else {
            return None;
        };
        if EXEC_METADATA_FIELDS.contains(&field.no) {
            return None;
        }
        Some(CursorExecRequest::Unknown {
            seq,
            field_no: field.no,
        })
    })
}
