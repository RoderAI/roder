use super::*;

pub(crate) fn encode_exec_mcp_result(
    seq: u64,
    text: &str,
    is_error: bool,
    data: &Value,
) -> Vec<u8> {
    let mut success = vec![
        proto_field_bytes(
            1,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_string(1, text)]),
            )]),
        ),
        proto_field_varint(2, u64::from(is_error)),
    ];
    if data.is_object() {
        success.push(proto_field_bytes(3, encode_protobuf_struct(data)));
    }
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            11,
            proto_message(vec![proto_field_bytes(1, proto_message(success))]),
        ),
    ]))
}

pub(crate) fn encode_exec_mcp_state_result(seq: u64, tools: &[CursorMcpTool]) -> Vec<u8> {
    let mut server = vec![
        proto_field_string(1, "Roder"),
        proto_field_string(2, "roder"),
        proto_field_string(7, "connected"),
    ];
    for tool in tools {
        server.push(proto_field_bytes(5, encode_mcp_tool_definition(tool)));
    }
    let success = proto_message(vec![proto_field_bytes(1, proto_message(server))]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(36, proto_message(vec![proto_field_bytes(1, success)])),
    ]))
}
