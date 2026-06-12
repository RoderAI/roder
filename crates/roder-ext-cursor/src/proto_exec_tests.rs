//! Offline fixtures for the Cursor same-stream exec channel (roadmap phase
//! 68, Stage 6): server-requested READ/WRITE/SHELL/SEARCH/REQUEST-CONTEXT
//! frames decode into typed exec requests with their seq and tool-call ids
//! preserved, and the client result encoders produce the exact
//! `ExecClientMessage` shapes the Cursor harness sends back on the same
//! stream.

use super::*;

fn exec_server_frame(seq: u64, field: u32, body: Vec<u8>) -> Vec<u8> {
    // AgentServerMessage { 2: ExecServerMessage { 1: seq, <field>: body } }
    proto_message(vec![proto_field_bytes(
        2,
        proto_message(vec![
            proto_field_varint(1, seq),
            proto_field_bytes(field, body),
        ]),
    )])
}

#[test]
fn decodes_exec_read_request_with_seq_and_tool_call_id() {
    let frame = exec_server_frame(
        41,
        7,
        proto_message(vec![
            proto_field_string(1, "src/main.rs"),
            proto_field_string(2, "tool_read_77"),
        ]),
    );

    let decoded = decode_server_frame(&frame);
    let Some(CursorExecRequest::Read {
        seq,
        path,
        tool_call_id,
    }) = decoded.exec
    else {
        panic!("expected READ exec request, got {:?}", decoded.exec);
    };
    assert_eq!(seq, 41);
    assert_eq!(path, "src/main.rs");
    assert_eq!(tool_call_id, "tool_read_77");
}

#[test]
fn decodes_exec_write_request_with_content_bytes() {
    let frame = exec_server_frame(
        42,
        3,
        proto_message(vec![
            proto_field_string(1, "notes.txt"),
            proto_field_bytes(2, b"new file body\n".to_vec()),
            proto_field_string(3, "tool_write_5"),
        ]),
    );

    let decoded = decode_server_frame(&frame);
    let Some(CursorExecRequest::Write {
        seq,
        path,
        content,
        tool_call_id,
    }) = decoded.exec
    else {
        panic!("expected WRITE exec request, got {:?}", decoded.exec);
    };
    assert_eq!(seq, 42);
    assert_eq!(path, "notes.txt");
    assert_eq!(content, b"new file body\n");
    assert_eq!(tool_call_id, "tool_write_5");
}

#[test]
fn decodes_exec_shell_request() {
    let frame = exec_server_frame(
        43,
        14,
        proto_message(vec![
            proto_field_string(1, "cargo check"),
            proto_field_string(2, "/repo"),
            proto_field_string(4, "tool_shell_9"),
        ]),
    );

    let decoded = decode_server_frame(&frame);
    let Some(CursorExecRequest::Shell {
        seq,
        command,
        cwd,
        tool_call_id,
    }) = decoded.exec
    else {
        panic!("expected SHELL exec request, got {:?}", decoded.exec);
    };
    assert_eq!(seq, 43);
    assert_eq!(command, "cargo check");
    assert_eq!(cwd, "/repo");
    assert_eq!(tool_call_id, "tool_shell_9");
}

#[test]
fn decodes_exec_search_grep_and_glob_variants() {
    // Claude-style grep: content pattern + files_with_matches result shape.
    let grep_frame = exec_server_frame(
        44,
        5,
        proto_message(vec![
            proto_field_string(1, "TODO"),
            proto_field_string(2, "crates"),
            proto_field_string(4, "files_with_matches"),
            proto_field_string(14, "tool_search_1"),
        ]),
    );
    let decoded = decode_server_frame(&grep_frame);
    let Some(CursorExecRequest::Search {
        seq,
        pattern,
        path,
        glob,
        mode,
        tool_call_id,
    }) = decoded.exec
    else {
        panic!("expected SEARCH exec request, got {:?}", decoded.exec);
    };
    assert_eq!(seq, 44);
    assert_eq!(pattern.as_deref(), Some("TODO"));
    assert_eq!(path, "crates");
    assert_eq!(glob, None);
    assert_eq!(mode, "files_with_matches");
    assert_eq!(tool_call_id, "tool_search_1");

    // Composer-style glob: path glob only, no content pattern.
    let glob_frame = exec_server_frame(
        45,
        5,
        proto_message(vec![
            proto_field_string(2, "."),
            proto_field_string(3, "**/*.rs"),
            proto_field_string(4, "files_with_matches"),
            proto_field_string(14, "tool_search_2"),
        ]),
    );
    let decoded = decode_server_frame(&glob_frame);
    let Some(CursorExecRequest::Search { pattern, glob, .. }) = decoded.exec else {
        panic!("expected SEARCH exec request");
    };
    assert_eq!(pattern, None);
    assert_eq!(glob.as_deref(), Some("**/*.rs"));
}

#[test]
fn decodes_unimplemented_exec_request_as_unknown_with_seq_and_field() {
    // An exec oneof slot Roder has no handler for (e.g. a delete/ls/todo
    // request). It must decode as Unknown — not None — so the bidi client can
    // reply; a silent drop left the server waiting and the turn stuck on the
    // tool call until the no-progress cap killed it (seen with composer-2.5).
    let frame = exec_server_frame(
        47,
        21,
        proto_message(vec![proto_field_string(1, "some/target/path")]),
    );

    let decoded = decode_server_frame(&frame);
    let Some(CursorExecRequest::Unknown {
        seq,
        field_no,
        payload,
    }) = decoded.exec
    else {
        panic!("expected UNKNOWN exec request, got {:?}", decoded.exec);
    };
    assert_eq!(seq, 47);
    assert_eq!(field_no, 21);
    assert_eq!(
        collect_payload_strings(&payload),
        vec!["some/target/path".to_string()]
    );
}

#[test]
fn exec_metadata_fields_do_not_decode_as_unknown_requests() {
    // f15 (message uuid) and f19 (routing) are envelope metadata, not the
    // request oneof; a frame carrying only those has no serviceable request.
    let frame = exec_server_frame(48, 15, b"00000000-0000-0000-0000-000000000000".to_vec());
    assert!(decode_server_frame(&frame).exec.is_none());
}

#[test]
fn malformed_known_exec_request_decodes_as_unknown_not_dropped() {
    // A SHELL request missing its command (field 1) used to abort decoding and
    // drop the frame entirely; it must now surface as Unknown so a result is
    // still sent.
    let frame = exec_server_frame(49, 14, proto_message(vec![proto_field_string(2, "/repo")]));
    let decoded = decode_server_frame(&frame);
    assert!(matches!(
        decoded.exec,
        Some(CursorExecRequest::Unknown {
            seq: 49,
            field_no: 14,
            ..
        })
    ));
}

#[test]
fn unknown_result_frame_mirrors_seq_and_request_field_number() {
    let frame = encode_exec_unknown_result(47, 21);
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(47), "seq must echo the request");
    assert_eq!(
        submessage(&exec, 21),
        Some(Vec::new()),
        "result must mirror the request field number with an empty body"
    );
}

#[test]
fn decodes_request_context_and_kv_put_frames() {
    let context_frame = exec_server_frame(46, 10, proto_message(vec![proto_field_varint(1, 1)]));
    let decoded = decode_server_frame(&context_frame);
    assert!(matches!(
        decoded.exec,
        Some(CursorExecRequest::RequestContext { id: 46 })
    ));

    // AgentServerMessage { 4: kv_server_message { 1: seq } } must be acked.
    let kv_frame = proto_message(vec![proto_field_bytes(
        4,
        proto_message(vec![proto_field_varint(1, 99)]),
    )]);
    let decoded = decode_server_frame(&kv_frame);
    assert_eq!(decoded.kv_seq, Some(99));

    let ack = encode_kv_ack(99);
    let kv_client = submessage(&ack, 3).expect("kv_client message");
    assert_eq!(scalar_u64(&kv_client, 1), Some(99));
}

fn exec_client_body(frame: &[u8]) -> Vec<u8> {
    submessage(frame, 2).expect("exec_client_message")
}

#[test]
fn read_result_frame_echoes_seq_and_carries_content_metadata() {
    let frame = encode_exec_read_result(41, "src/main.rs", b"fn main() {}\n", 1);
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(41), "seq must match the request");
    let read = submessage(&exec, 7).expect("read result");
    let inner = submessage(&read, 1).expect("read payload");
    assert_eq!(scalar_string(&inner, 1).as_deref(), Some("src/main.rs"));
    assert_eq!(scalar_string(&inner, 2).as_deref(), Some("fn main() {}\n"));
    assert_eq!(scalar_u64(&inner, 3), Some(1));
    assert_eq!(scalar_u64(&inner, 4), Some(13));
}

#[test]
fn write_result_frame_echoes_seq_path_lines_and_size() {
    let frame = encode_exec_write_result(42, "notes.txt", 3, 27);
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(42));
    let write = submessage(&exec, 3).expect("write result");
    let inner = submessage(&write, 1).expect("write payload");
    assert_eq!(scalar_string(&inner, 1).as_deref(), Some("notes.txt"));
    assert_eq!(scalar_u64(&inner, 2), Some(3));
    assert_eq!(scalar_u64(&inner, 3), Some(27));
}

#[test]
fn shell_result_streams_start_stdout_and_exit_frames_with_one_seq() {
    let frames = encode_exec_shell_results(43, "/repo", "ok\n");
    assert_eq!(frames.len(), 3, "start, stdout, exit");
    for frame in &frames {
        let exec = exec_client_body(frame);
        assert_eq!(scalar_u64(&exec, 1), Some(43));
        assert!(submessage(&exec, 14).is_some(), "shell result field");
    }
    // stdout frame: 14:{ 1:{ 1:stdout } }
    let stdout_exec = exec_client_body(&frames[1]);
    let shell = submessage(&stdout_exec, 14).unwrap();
    let out = submessage(&shell, 1).expect("stdout body");
    assert_eq!(scalar_string(&out, 1).as_deref(), Some("ok\n"));
    // exit frame: 14:{ 3:{ 2:cwd, 6:bytes } }
    let exit_exec = exec_client_body(&frames[2]);
    let shell = submessage(&exit_exec, 14).unwrap();
    let exit = submessage(&shell, 3).expect("exit body");
    assert_eq!(scalar_string(&exit, 2).as_deref(), Some("/repo"));
    assert_eq!(scalar_u64(&exit, 6), Some(3));
}

#[test]
fn glob_result_lists_relative_paths_under_requested_root() {
    let frame = encode_exec_glob_result(
        45,
        ".",
        "/repo",
        &["src/lib.rs".to_string(), "src/main.rs".to_string()],
    );
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(45));
    let search = submessage(&exec, 5).expect("search result");
    let inner = submessage(&search, 1).expect("search payload");
    assert_eq!(scalar_string(&inner, 2).as_deref(), Some("."));
    assert_eq!(
        scalar_string(&inner, 3).as_deref(),
        Some("files_with_matches")
    );
    let f4 = submessage(&inner, 4).expect("result body");
    assert_eq!(scalar_string(&f4, 1).as_deref(), Some("/repo"));
    let files = submessage(&f4, 2)
        .and_then(|body| submessage(&body, 2))
        .expect("file list");
    assert_eq!(scalar_string(&files, 1).as_deref(), Some("src/lib.rs"));
    assert_eq!(scalar_u64(&files, 2), Some(2), "match count");
}

#[test]
fn grep_result_carries_line_matches_and_counts() {
    let matches = vec![CursorGrepMatch {
        path: "src/lib.rs".to_string(),
        line: 7,
        text: "// TODO: fix".to_string(),
    }];
    let frame = encode_exec_grep_result(44, "TODO", "crates", "/repo", &matches);
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(44));
    let search = submessage(&exec, 5).expect("search result");
    let inner = submessage(&search, 1).expect("search payload");
    assert_eq!(scalar_string(&inner, 1).as_deref(), Some("TODO"));
    assert_eq!(scalar_string(&inner, 3).as_deref(), Some("content"));
    let f4 = submessage(&inner, 4).expect("result body");
    let entries = submessage(&f4, 2)
        .and_then(|body| submessage(&body, 3))
        .expect("match entries");
    let entry = submessage(&entries, 1).expect("first match");
    assert_eq!(scalar_string(&entry, 1).as_deref(), Some("src/lib.rs"));
    let line = submessage(&entry, 2).expect("line body");
    assert_eq!(scalar_u64(&line, 1), Some(7));
    assert_eq!(scalar_string(&line, 2).as_deref(), Some("// TODO: fix"));
    assert_eq!(scalar_u64(&entries, 2), Some(1));
    assert_eq!(scalar_u64(&entries, 3), Some(1));
}

#[test]
fn request_context_result_unblocks_generation_with_workspace_env() {
    let frame = encode_exec_request_context_result(46, "/repo");
    let exec = exec_client_body(&frame);
    assert_eq!(scalar_u64(&exec, 1), Some(46), "id must echo the request");
    let env = submessage(&exec, 10)
        .and_then(|result| submessage(&result, 1))
        .and_then(|success| submessage(&success, 1))
        .and_then(|context| submessage(&context, 4))
        .expect("request-context env");
    assert_eq!(scalar_string(&env, 2).as_deref(), Some("/repo"));
    assert_eq!(scalar_string(&env, 11).as_deref(), Some("/repo"));
}
