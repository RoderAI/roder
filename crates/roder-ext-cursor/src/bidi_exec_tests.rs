use super::*;
use std::sync::Mutex;

struct Executor {
    calls: Mutex<Vec<ToolCallCompleted>>,
    data: Value,
    is_error: bool,
}
#[async_trait::async_trait]
impl TurnToolExecutor for Executor {
    async fn execute(&self, call: ToolCallCompleted) -> anyhow::Result<TurnToolOutcome> {
        self.calls.lock().unwrap().push(call);
        Ok(TurnToolOutcome {
            result: "executor output".into(),
            data: self.data.clone(),
            is_error: self.is_error,
        })
    }
}
fn executor(data: Value, is_error: bool) -> Executor {
    Executor {
        calls: Mutex::new(vec![]),
        data,
        is_error,
    }
}
fn result(frames: &[Vec<u8>], field: u32) -> Vec<u8> {
    submessage(&submessage(&frames[0], 2).unwrap(), field).unwrap()
}
#[tokio::test]
async fn denied_write_and_delete_never_touch_local_files() {
    let path = std::env::temp_dir().join(format!("cursor-policy-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, "original").unwrap();
    let executor = executor(Value::Null, true);
    for request in [
        CursorExecRequest::Write {
            seq: 1,
            path: path.to_string_lossy().into_owned(),
            content: b"unauthorized".to_vec(),
            tool_call_id: "write".into(),
        },
        CursorExecRequest::Delete {
            seq: 2,
            path: path.to_string_lossy().into_owned(),
            tool_call_id: "delete".into(),
        },
    ] {
        let (frames, ack) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
            .await
            .unwrap();
        assert!(ack.is_some());
        let exec = submessage(&frames[0], 2).unwrap();
        let reply = submessage(&exec, 3)
            .or_else(|| submessage(&exec, 4))
            .unwrap();
        assert!(
            submessage(&reply, 1).is_none(),
            "denial cannot become success"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    }
    std::fs::remove_file(path).unwrap();
}
#[tokio::test]
async fn read_uses_remote_executor_content_and_forwards_range() {
    let executor = executor(
        json!({"content":"remote line\n","total_lines":20,"file_size":200,"truncated":true}),
        false,
    );
    let request = CursorExecRequest::Read {
        seq: 3,
        field_no: 29,
        path: "remote-only.txt".into(),
        tool_call_id: "read".into(),
        offset: Some(5),
        limit: Some(1),
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
        .await
        .unwrap();
    let success = submessage(&result(&frames, 29), 1).unwrap();
    assert_eq!(scalar_string(&success, 2).as_deref(), Some("remote line\n"));
    assert_eq!(scalar_u64(&success, 4), Some(200));
    assert_eq!(scalar_u64(&success, 8), Some(1));
    let args: Value = serde_json::from_str(&executor.calls.lock().unwrap()[0].arguments).unwrap();
    assert_eq!(args["start_line"], 5);
    assert_eq!(args["limit"], 1);
}
#[tokio::test]
async fn shell_nonzero_exit_survives_stream_and_unary_replies() {
    let executor = executor(
        json!({"exit_code":7,"aggregated_output":"failure details","duration_ms":12}),
        true,
    );
    for field_no in [2, 14] {
        let request = CursorExecRequest::Shell {
            seq: 4,
            field_no,
            command: "exit 7".into(),
            cwd: ".".into(),
            tool_call_id: "shell".into(),
            timeout_ms: Some(2000),
        };
        let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
            .await
            .unwrap();
        if field_no == 2 {
            let failure = submessage(&result(&frames, 2), 2).unwrap();
            assert_eq!(scalar_u64(&failure, 3), Some(7));
        } else {
            let stream = submessage(&submessage(frames.last().unwrap(), 2).unwrap(), 14).unwrap();
            assert_eq!(scalar_u64(&submessage(&stream, 3).unwrap(), 1), Some(7));
        }
    }
}
#[tokio::test]
async fn relative_search_returns_executor_matches_and_regex_options() {
    let executor = executor(json!({"matches":["src/a.rs:12:Alpha=42"]}), false);
    let request = CursorExecRequest::Search {
        seq: 5,
        pattern: Some("alpha=\\d+".into()),
        path: "src".into(),
        glob: Some("*.rs".into()),
        mode: "content".into(),
        tool_call_id: "grep".into(),
        case_insensitive: true,
        multiline: false,
        offset: 0,
        limit: Some(10),
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
        .await
        .unwrap();
    let success = submessage(&result(&frames, 5), 1).unwrap();
    assert!(String::from_utf8_lossy(&success).contains("Alpha=42"));
    let args: Value = serde_json::from_str(&executor.calls.lock().unwrap()[0].arguments).unwrap();
    assert_eq!(args["path"], "src");
    assert_eq!(args["regex"], true);
    assert_eq!(args["case_sensitive"], false);
}
#[tokio::test]
async fn mcp_arguments_and_errors_are_preserved() {
    let executor = executor(json!({"details":"failure"}), true);
    let request = CursorExecRequest::Mcp {
        seq: 6,
        name: "custom_tool".into(),
        tool_call_id: "mcp".into(),
        arguments: json!({"nested":[true,3,"x"]}),
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
        .await
        .unwrap();
    let success = submessage(&result(&frames, 11), 1).unwrap();
    assert_eq!(scalar_u64(&success, 2), Some(1));
    assert_eq!(executor.calls.lock().unwrap()[0].name, "custom_tool");
}
#[tokio::test]
async fn missing_executor_is_an_error_not_empty_file_success() {
    let request = CursorExecRequest::Read {
        seq: 7,
        field_no: 7,
        path: "missing".into(),
        tool_call_id: "read".into(),
        offset: None,
        limit: None,
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), None, &[])
        .await
        .unwrap();
    assert!(submessage(&result(&frames, 7), 2).is_some());
}

#[tokio::test]
async fn missing_file_returns_native_not_found_variant() {
    let executor = executor(
        json!({"error":{"kind":"not_found","message":"path does not exist"}}),
        true,
    );
    let request = CursorExecRequest::Read {
        seq: 8,
        field_no: 7,
        path: "new.txt".into(),
        tool_call_id: "read".into(),
        offset: None,
        limit: None,
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
        .await
        .unwrap();
    let missing = submessage(&result(&frames, 7), 4)
        .expect("native missing-file response is required for create-file flow");
    assert_eq!(scalar_string(&missing, 1).as_deref(), Some("new.txt"));
}
#[tokio::test]
async fn listing_encodes_files_and_directories() {
    let executor = executor(json!({"entries":["src/","main.rs"]}), false);
    let request = CursorExecRequest::List {
        seq: 9,
        path: "project".into(),
        tool_call_id: "ls".into(),
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), Some(&executor), &[])
        .await
        .unwrap();
    let tree = submessage(&submessage(&result(&frames, 8), 1).unwrap(), 1).unwrap();
    assert_eq!(scalar_string(&tree, 1).as_deref(), Some("/remote/project"));
    assert_eq!(scalar_u64(&tree, 6), Some(1));
    assert!(submessage(&tree, 2).is_some());
    assert!(submessage(&tree, 3).is_some());
}

#[tokio::test]
async fn mcp_state_lists_runtime_tool_definitions() {
    let tools = vec![CursorMcpTool {
        name: "update_plan".into(),
        description: "Track a plan".into(),
        input_schema: json!({"type":"object"}),
    }];
    let request = CursorExecRequest::McpState {
        seq: 10,
        server_identifiers: vec!["roder".into()],
    };
    let (frames, _) = service_exec(request, Path::new("/remote"), None, &tools)
        .await
        .unwrap();
    let server = submessage(&submessage(&result(&frames, 36), 1).unwrap(), 1).unwrap();
    assert_eq!(scalar_string(&server, 2).as_deref(), Some("roder"));
    let definition = submessage(&server, 5).unwrap();
    assert_eq!(
        scalar_string(&definition, 5).as_deref(),
        Some("update_plan")
    );
}
