use super::*;

pub(super) fn wrap_exec_client(exec: Vec<u8>) -> Vec<u8> {
    // AgentClientMessage { 2: exec_client_message }
    proto_message(vec![proto_field_bytes(2, exec)])
}

/// READ result: ExecClientMessage{ 1:seq, 7:{ 1:{ 1:path, 2:content, 3:total_lines, 4:file_size } } }
pub(crate) fn encode_exec_read_result(
    seq: u64,
    field_no: u32,
    path: &str,
    content: &[u8],
    total_lines: u64,
    file_size: u64,
    truncated: bool,
    range_applied: bool,
) -> Vec<u8> {
    let inner = proto_message(vec![
        proto_field_string(1, path),
        proto_field_bytes(2, content.to_vec()),
        proto_field_varint(3, total_lines),
        proto_field_varint(4, file_size),
        proto_field_varint(6, u64::from(truncated)),
        proto_field_varint(8, u64::from(range_applied)),
    ]);
    let read = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(field_no, read),
    ]))
}

/// WRITE result: ExecClientMessage{ 1:seq, 3:{ 1:{ 1:path, 2:lines, 3:size } } }
pub(crate) fn encode_exec_write_result(
    seq: u64,
    path: &str,
    lines: u64,
    size: u64,
    content: &str,
) -> Vec<u8> {
    let inner = proto_message(vec![
        proto_field_string(1, path),
        proto_field_varint(2, lines),
        proto_field_varint(3, size),
        proto_field_string(4, content),
    ]);
    let write = proto_message(vec![proto_field_bytes(1, inner)]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(3, write),
    ]))
}

/// SHELL result, streamed as three ExecClientMessages with the same seq,
/// matching an authoritative cursor-agent v2026.06.12 wire capture of a
/// successful `wc -c` run:
///   start:  14:{ 4:{ 1:{ 1:1 } } }
///   stdout: 14:{ 1:{ 1:chunk } }
///   exit:   14:{ 3:{ 2:cwd, 6:duration_ms } }
///
/// `cwd` must be non-empty (cursor-agent always sends the real working
/// directory) and f6 of the exit message is the command duration in
/// milliseconds, NOT an exit code or byte count — the capture shows f6=29 for
/// a command that exited 0 and printed 25 bytes.
pub(crate) fn encode_exec_shell_results(
    seq: u64,
    cwd: &str,
    stdout: &str,
    duration_ms: u64,
    exit_code: i32,
) -> Vec<Vec<u8>> {
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
    // stdout: 14:{ 1:{ 1:chunk } }
    let out = shell_msg(proto_message(vec![proto_field_bytes(
        1,
        proto_message(vec![proto_field_string(1, stdout)]),
    )]));
    // exit: 14:{ 3:{ 2:cwd, 6:duration_ms } }
    let exit = shell_msg(proto_message(vec![proto_field_bytes(
        3,
        proto_message(vec![
            proto_field_varint(1, exit_code.max(0) as u64),
            proto_field_string(2, cwd),
            proto_field_varint(6, duration_ms),
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
    truncated: bool,
) -> Vec<u8> {
    let mut files: Vec<Vec<u8>> = rel_paths.iter().map(|p| proto_field_string(1, p)).collect();
    files.push(proto_field_varint(2, rel_paths.len() as u64));
    files.push(proto_field_varint(3, u64::from(truncated)));
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
    truncated: bool,
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
    entries.push(proto_field_varint(4, u64::from(truncated)));
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

/// Explicit protocol failure for an unsupported request, preserving its id.
pub(crate) fn encode_exec_unknown_result(seq: u64, field_no: u32) -> Vec<u8> {
    // ExecClientThrow is the schema-defined failure channel. Unknown request
    // and response field numbers are not necessarily symmetric (e.g. Pi tools).
    proto_message(vec![proto_field_bytes(
        5,
        proto_message(vec![proto_field_bytes(
            2,
            proto_message(vec![
                proto_field_varint(1, seq),
                proto_field_string(
                    2,
                    &format!("unsupported Cursor exec request field {field_no}"),
                ),
                proto_field_string(4, "UNSUPPORTED_EXEC_REQUEST"),
            ]),
        )]),
    )])
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
        proto_field_string(2, workspace_path), // workspace_paths[]
        proto_field_string(11, workspace_path), // project_folder
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

/// exec_client_control_message ack: AgentClientMessage{ 5:{ 1:{ 1:seq } } }.
///
/// The ack must echo the exec seq it acknowledges (authoritative cursor-agent
/// capture: `0a020801` = {1:{1:1}} after servicing exec seq 1). The previous
/// empty `{1:<empty>}` ack carried no seq, so after a *streamed* shell result
/// the server never learned the exec was finished and did not resume the model
/// — the turn sat "stuck on the shell tool" until the no-progress cap ended it.
pub(crate) fn encode_exec_control(seq: u64) -> Vec<u8> {
    proto_message(vec![proto_field_bytes(
        5,
        proto_message(vec![proto_field_bytes(
            1,
            proto_message(vec![proto_field_varint(1, seq)]),
        )]),
    )])
}

pub(crate) fn encode_exec_error(
    seq: u64,
    field_no: u32,
    path: &str,
    error: &str,
    kind: &str,
) -> Vec<u8> {
    let path_error_field = match (field_no, kind) {
        (7 | 29, "not_found") => Some(4),
        (7 | 29, "permission_denied") => Some(5),
        (3, "permission_denied") => Some(3),
        (4, "not_found") => Some(2),
        (4, "permission_denied") => Some(4),
        _ => None,
    };
    if let Some(error_field) = path_error_field {
        return wrap_exec_client(proto_message(vec![
            proto_field_varint(1, seq),
            proto_field_bytes(
                field_no,
                proto_message(vec![proto_field_bytes(
                    error_field,
                    proto_message(vec![proto_field_string(1, path)]),
                )]),
            ),
        ]));
    }
    let (result_field, body) = match field_no {
        7 | 29 | 8 | 9 => (
            2,
            proto_message(vec![
                proto_field_string(1, path),
                proto_field_string(2, error),
            ]),
        ),
        3 => (
            5,
            proto_message(vec![
                proto_field_string(1, path),
                proto_field_string(2, error),
            ]),
        ),
        4 => (
            7,
            proto_message(vec![
                proto_field_string(1, path),
                proto_field_string(2, error),
            ]),
        ),
        5 | 11 | 36 => (2, proto_message(vec![proto_field_string(1, error)])),
        14 => (
            5,
            proto_message(vec![
                proto_field_string(2, path),
                proto_field_string(3, error),
            ]),
        ),
        2 => (
            4,
            proto_message(vec![
                proto_field_string(2, path),
                proto_field_string(3, error),
            ]),
        ),
        _ => return encode_exec_unknown_result(seq, field_no),
    };
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            field_no,
            proto_message(vec![proto_field_bytes(result_field, body)]),
        ),
    ]))
}

pub(crate) fn encode_exec_shell_result(
    seq: u64,
    command: &str,
    cwd: &str,
    output: &str,
    duration: u64,
    exit_code: i32,
) -> Vec<u8> {
    let body = proto_message(vec![
        proto_field_string(1, command),
        proto_field_string(2, cwd),
        proto_field_varint(3, exit_code.max(0) as u64),
        proto_field_string(5, output),
        proto_field_varint(7, duration),
    ]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(
                if exit_code == 0 { 1 } else { 2 },
                body,
            )]),
        ),
    ]))
}

pub(crate) fn encode_exec_list_result(seq: u64, path: &str, entries: &[String]) -> Vec<u8> {
    let mut tree = vec![proto_field_string(1, path), proto_field_varint(4, 1)];
    let mut files = 0;
    for entry in entries {
        if entry.ends_with('/') {
            tree.push(proto_field_bytes(
                2,
                proto_message(vec![proto_field_string(
                    1,
                    &std::path::Path::new(path)
                        .join(entry.trim_end_matches('/'))
                        .to_string_lossy(),
                )]),
            ));
        } else {
            files += 1;
            tree.push(proto_field_bytes(
                3,
                proto_message(vec![proto_field_string(1, entry)]),
            ));
        }
    }
    tree.push(proto_field_varint(6, files));
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            8,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_bytes(1, proto_message(tree))]),
            )]),
        ),
    ]))
}

pub(crate) fn encode_exec_count_result(
    seq: u64,
    pattern: &str,
    path: &str,
    root: &str,
    matches: &[CursorGrepMatch],
    truncated: bool,
) -> Vec<u8> {
    let mut counts = BTreeMap::<&str, u64>::new();
    for item in matches {
        *counts.entry(&item.path).or_default() += 1;
    }
    let mut fields = vec![
        proto_field_varint(2, counts.len() as u64),
        proto_field_varint(3, matches.len() as u64),
        proto_field_varint(4, u64::from(truncated)),
    ];
    for (file, count) in counts {
        fields.push(proto_field_bytes(
            1,
            proto_message(vec![
                proto_field_string(1, file),
                proto_field_varint(2, count),
            ]),
        ));
    }
    let workspace = proto_message(vec![
        proto_field_string(1, root),
        proto_field_bytes(
            2,
            proto_message(vec![proto_field_bytes(1, proto_message(fields))]),
        ),
    ]);
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            5,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![
                    proto_field_string(1, pattern),
                    proto_field_string(2, path),
                    proto_field_string(3, "count"),
                    proto_field_bytes(4, workspace),
                ]),
            )]),
        ),
    ]))
}

pub(crate) fn encode_exec_delete_result(seq: u64, path: &str) -> Vec<u8> {
    wrap_exec_client(proto_message(vec![
        proto_field_varint(1, seq),
        proto_field_bytes(
            4,
            proto_message(vec![proto_field_bytes(
                1,
                proto_message(vec![proto_field_string(1, path)]),
            )]),
        ),
    ]))
}
