use claude_code_sdk_rust::{
    AssistantContent, ContentBlock, Message, TaskUpdatedStatus, UserContent,
};
use serde_json::json;

use claude_code_sdk_rust::internal::parser::parse_message_value;

#[test]
fn deserializes_user_message_from_cli_wire_shape() {
    let raw = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": "hello"
        },
        "parent_tool_use_id": null,
        "session_id": "default",
        "uuid": "user-1"
    });

    let parsed: Message = serde_json::from_value(raw).expect("valid user message");

    match parsed {
        Message::UserMsg {
            content:
                UserContent {
                    role,
                    content: serde_json::Value::String(text),
                },
            uuid,
            parent_tool_use_id,
            ..
        } => {
            assert_eq!(role, Some("user".to_string()));
            assert_eq!(text, "hello");
            assert_eq!(uuid, Some("user-1".to_string()));
            assert_eq!(parent_tool_use_id, None);
        }
        other => panic!("expected user message, got {other:?}"),
    }
}

#[test]
fn deserializes_assistant_message_with_nested_metadata_and_server_tools() {
    let raw = json!({
        "type": "assistant",
        "message": {
            "id": "msg_1",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 2},
            "content": [
                {"type": "text", "text": "hi"},
                {"type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {"query": "rust"}},
                {"type": "advisor_tool_result", "tool_use_id": "srv_1", "content": {"type": "web_search_result"}}
            ]
        },
        "session_id": "sess_1",
        "uuid": "assistant-1"
    });

    let parsed: Message = serde_json::from_value(raw).expect("valid assistant message");

    match parsed {
        Message::AssistantMsg {
            content:
                AssistantContent {
                    id,
                    model,
                    stop_reason,
                    usage,
                    content,
                },
            session_id,
            uuid,
            ..
        } => {
            assert_eq!(id, Some("msg_1".to_string()));
            assert_eq!(model, "claude-sonnet-4-5");
            assert_eq!(stop_reason, Some("end_turn".to_string()));
            assert_eq!(
                usage.unwrap().get("output_tokens").and_then(|v| v.as_i64()),
                Some(2)
            );
            assert_eq!(session_id, Some("sess_1".to_string()));
            assert_eq!(uuid, Some("assistant-1".to_string()));
            assert!(matches!(content[0], ContentBlock::Text { .. }));
            assert!(matches!(content[1], ContentBlock::ServerToolUse { .. }));
            assert!(matches!(content[2], ContentBlock::ServerToolResult { .. }));
        }
        other => panic!("expected assistant message, got {other:?}"),
    }
}

#[test]
fn deserializes_result_message_current_fields() {
    let raw = json!({
        "type": "result",
        "subtype": "success",
        "duration_ms": 10,
        "duration_api_ms": 8,
        "is_error": false,
        "num_turns": 1,
        "session_id": "sess_1",
        "stop_reason": "end_turn",
        "modelUsage": {"claude-sonnet-4-5": {"input_tokens": 1}},
        "deferred_tool_use": {"id": "toolu_1", "name": "Bash", "input": {"command": "pwd"}},
        "errors": ["recoverable"],
        "api_error_status": 429,
        "uuid": "result-1"
    });

    let parsed: Message = serde_json::from_value(raw).expect("valid result message");

    match parsed {
        Message::ResultMsg {
            model_usage,
            deferred_tool_use,
            errors,
            api_error_status,
            uuid,
            ..
        } => {
            assert!(model_usage.unwrap().contains_key("claude-sonnet-4-5"));
            assert_eq!(deferred_tool_use.unwrap().name, "Bash");
            assert_eq!(errors, Some(vec!["recoverable".to_string()]));
            assert_eq!(api_error_status, Some(429));
            assert_eq!(uuid, Some("result-1".to_string()));
        }
        other => panic!("expected result message, got {other:?}"),
    }
}

#[test]
fn parser_skips_unknown_message_types_for_forward_compatibility() {
    let raw = json!({
        "type": "future_message",
        "payload": {"kept": true}
    });

    let parsed = parse_message_value(raw).expect("unknown messages are not parse errors");

    assert!(parsed.is_none());
}

#[test]
fn parser_returns_typed_task_system_messages() {
    let raw = json!({
        "type": "system",
        "subtype": "task_progress",
        "task_id": "task-abc",
        "description": "Halfway",
        "usage": {"total_tokens": 123, "tool_uses": 2, "duration_ms": 456},
        "uuid": "uuid-1",
        "session_id": "session-1",
        "tool_use_id": "toolu_1",
        "last_tool_name": "Read"
    });

    let parsed = parse_message_value(raw)
        .expect("valid task message")
        .unwrap();

    match parsed {
        Message::TaskProgressMsg(message) => {
            assert_eq!(message.task_id, "task-abc");
            assert_eq!(message.usage.total_tokens, 123);
            assert_eq!(message.tool_use_id.as_deref(), Some("toolu_1"));
            assert_eq!(message.last_tool_name.as_deref(), Some("Read"));
        }
        other => panic!("expected task progress message, got {other:?}"),
    }
}

#[test]
fn parser_returns_typed_task_updated_message_with_status_from_patch() {
    let raw = json!({
        "type": "system",
        "subtype": "task_updated",
        "task_id": "task-xyz",
        "patch": {"status": "killed", "end_time": 1234},
        "session_id": "session-1",
        "uuid": "uuid-9"
    });

    let parsed = parse_message_value(raw)
        .expect("valid task_updated message")
        .unwrap();

    match parsed {
        Message::TaskUpdatedMsg(message) => {
            assert_eq!(message.task_id, "task-xyz");
            // status is derived from patch.status, not a top-level field.
            assert_eq!(message.status, Some(TaskUpdatedStatus::Killed));
            assert_eq!(message.patch["end_time"], 1234);
            assert_eq!(message.session_id.as_deref(), Some("session-1"));
            assert_eq!(message.uuid.as_deref(), Some("uuid-9"));
            // `killed` is terminal across both lifecycle vocabularies.
            assert!(claude_code_sdk_rust::is_terminal_task_status("killed"));
        }
        other => panic!("expected task_updated message, got {other:?}"),
    }
}

#[test]
fn parser_task_updated_tolerates_missing_status_and_ids() {
    let raw = json!({
        "type": "system",
        "subtype": "task_updated",
        "task_id": "task-xyz",
        "patch": {"end_time": 1234}
    });

    let parsed = parse_message_value(raw)
        .expect("valid task_updated message")
        .unwrap();

    match parsed {
        Message::TaskUpdatedMsg(message) => {
            assert_eq!(message.task_id, "task-xyz");
            assert_eq!(message.status, None);
            assert_eq!(message.session_id, None);
            assert_eq!(message.uuid, None);
        }
        other => panic!("expected task_updated message, got {other:?}"),
    }
}

#[test]
fn parser_returns_typed_hook_system_messages() {
    let raw = json!({
        "type": "system",
        "subtype": "hook_response",
        "hook_event": "PostToolUse",
        "session_id": "session-1",
        "uuid": "uuid-1",
        "output": "",
        "exit_code": 0,
        "outcome": "success"
    });

    let parsed = parse_message_value(raw)
        .expect("valid hook message")
        .unwrap();

    match parsed {
        Message::HookEventMsg(message) => {
            assert_eq!(message.subtype, "hook_response");
            assert_eq!(message.hook_event_name.as_deref(), Some("PostToolUse"));
            assert_eq!(message.session_id.as_deref(), Some("session-1"));
            assert_eq!(message.data["outcome"], "success");
        }
        other => panic!("expected hook event message, got {other:?}"),
    }
}

#[test]
fn parser_returns_typed_mirror_error_messages() {
    let raw = json!({
        "type": "system",
        "subtype": "mirror_error",
        "key": {"project_key": "proj", "session_id": "sess"},
        "error": "disk full"
    });

    let parsed = parse_message_value(raw)
        .expect("valid mirror error")
        .unwrap();

    match parsed {
        Message::MirrorErrorMsg(message) => {
            assert_eq!(message.error, "disk full");
            assert_eq!(
                message.key.as_ref().and_then(|key| key.get("session_id")),
                Some(&json!("sess"))
            );
            assert_eq!(message.data["subtype"], "mirror_error");
        }
        other => panic!("expected mirror error message, got {other:?}"),
    }
}

#[test]
fn parser_accepts_python_rate_limit_wire_values() {
    let raw = json!({
        "type": "rate_limit_event",
        "rate_limit_info": {
            "status": "allowed_warning",
            "resetsAt": 1700000000,
            "rateLimitType": "five_hour",
            "utilization": 0.91
        },
        "uuid": "abc-123",
        "session_id": "session_xyz"
    });

    let parsed = parse_message_value(raw)
        .expect("valid rate limit event")
        .unwrap();

    match parsed {
        Message::RateLimitEventMsg {
            rate_limit_info,
            uuid,
            session_id,
        } => {
            assert_eq!(uuid, "abc-123");
            assert_eq!(session_id, "session_xyz");
            assert_eq!(rate_limit_info.resets_at, Some(1700000000));
            assert_eq!(rate_limit_info.utilization, Some(0.91));
        }
        other => panic!("expected rate limit event, got {other:?}"),
    }
}

#[test]
fn parser_accepts_string_tool_use_result_from_builtin_tool() {
    // The CLI sometimes echoes a tool result as a bare string (e.g. a
    // built-in tool's error text) instead of a structured object. The
    // parser must tolerate this rather than failing with a serde
    // "expected a map" error.
    let raw = json!({
        "type": "user",
        "message": {"role": "user", "content": "ack"},
        "tool_use_result": "Error: File does not exist. Note: your current working directory is /tmp",
        "parent_tool_use_id": "toolu_42"
    });

    let parsed = parse_message_value(raw)
        .expect("string tool_use_result must not be a parse error")
        .unwrap();

    match parsed {
        Message::UserMsg {
            tool_use_result, ..
        } => {
            assert_eq!(
                tool_use_result.and_then(|value| value.as_str().map(str::to_string)),
                Some(
                    "Error: File does not exist. Note: your current working directory is /tmp"
                        .to_string()
                )
            );
        }
        other => panic!("expected user message, got {other:?}"),
    }
}
