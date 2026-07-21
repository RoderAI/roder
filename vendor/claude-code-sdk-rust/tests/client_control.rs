use async_trait::async_trait;
use claude_code_sdk_rust::internal::transport::Transport;
use claude_code_sdk_rust::session_store::{InMemorySessionStore, SessionKey, SessionStore};
use claude_code_sdk_rust::{ClaudeAgentClient, ClaudeAgentOptions, Message, Result, StreamEvent};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MockState {
    connected: usize,
    closed: usize,
    writes: Vec<Vec<u8>>,
    scripted_messages: VecDeque<Vec<u8>>,
    initialized: bool,
    auto_control_responses: bool,
    auto_response_cursor: usize,
}

#[tokio::test]
async fn client_consumes_transcript_mirror_frames_without_yielding_them() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let store = InMemorySessionStore::new();
    let temp = std::env::temp_dir().join(format!(
        "claude-rust-client-mirror-test-{}",
        uuid::Uuid::new_v4()
    ));
    let projects_dir = temp.join("projects");
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "transcript_mirror",
                "filePath": projects_dir.join("proj/session-1.jsonl"),
                "entries": [{
                    "type": "user",
                    "uuid": "entry-1",
                    "message": {"content": "stored prompt"}
                }]
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "hello"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default",
                "stop_reason": "end_turn"
            }))
            .unwrap(),
        );
    }

    let mut env = std::collections::HashMap::new();
    env.insert(
        "CLAUDE_CONFIG_DIR".to_string(),
        temp.to_string_lossy().to_string(),
    );
    let options = ClaudeAgentOptions::builder()
        .env(env)
        .session_store(store.clone())
        .build();
    let mut client = ClaudeAgentClient::with_transport(options, Box::new(transport)).unwrap();

    client.connect().await.unwrap();
    client.query("hi").await.unwrap();
    let messages = client.receive_response().await.unwrap();

    assert_eq!(messages.len(), 2);
    let entries = store
        .load(SessionKey {
            project_key: "proj".to_string(),
            session_id: "session-1".to_string(),
            subpath: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["uuid"], "entry-1");
}

#[derive(Clone, Default)]
struct MockTransport {
    state: Arc<Mutex<MockState>>,
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
        self.state.lock().unwrap().connected += 1;
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.state.lock().unwrap().writes.push(data.to_vec());
        Ok(())
    }

    async fn close_input(&mut self) -> Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        if !state.initialized {
            let first_write = state.writes.first().expect("initialize write");
            let initialize: serde_json::Value = serde_json::from_slice(first_write)?;
            let request_id = initialize["request_id"].as_str().unwrap();
            state.initialized = true;
            return Ok(Some(
                serde_json::to_vec(&serde_json::json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": request_id,
                        "response": {"commands": []}
                    }
                }))
                .unwrap(),
            ));
        }
        if state.auto_control_responses {
            while state.auto_response_cursor < state.writes.len() {
                let index = state.auto_response_cursor;
                state.auto_response_cursor += 1;
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&state.writes[index])
                else {
                    continue;
                };
                if value.get("type").and_then(|value| value.as_str()) != Some("control_request") {
                    continue;
                }
                let Some(request_id) = value.get("request_id").and_then(|value| value.as_str())
                else {
                    continue;
                };
                return Ok(Some(
                    serde_json::to_vec(&serde_json::json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": request_id,
                            "response": {}
                        }
                    }))
                    .unwrap(),
                ));
            }
        }
        Ok(state.scripted_messages.pop_front())
    }

    async fn close(&mut self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.initialized = false;
        state.closed += 1;
        Ok(())
    }
}

#[tokio::test]
async fn spawned_stream_receiver_drop_closes_owned_transport() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    let task = tokio::spawn(async move {
        ClaudeAgentClient::run_client_stream(client, "hi".into(), tx)
            .await
            .expect("stream owner should close cleanly after receiver drop");
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if state.lock().unwrap().connected == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("client should connect");

    drop(rx);
    tokio::time::timeout(std::time::Duration::from_secs(1), task)
        .await
        .expect("receiver drop should unblock the owned stream task")
        .expect("stream task should not panic");
    assert_eq!(state.lock().unwrap().closed, 1);
}

#[tokio::test]
async fn client_connects_initializes_sends_query_and_receives_response() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "hello"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default",
                "stop_reason": "end_turn"
            }))
            .unwrap(),
        );
    }

    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    client.query("hi").await.expect("query");
    let messages = client.receive_response().await.expect("messages");

    assert_eq!(messages.len(), 2);
    assert!(matches!(messages[0], Message::AssistantMsg { .. }));
    assert!(matches!(messages[1], Message::ResultMsg { .. }));

    let state = state.lock().unwrap();
    assert_eq!(state.connected, 1);
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    assert_eq!(writes[0]["type"], "control_request");
    assert_eq!(writes[0]["request"]["subtype"], "initialize");
    assert_eq!(writes[1]["type"], "user");
    assert_eq!(writes[1]["message"]["content"], "hi");
}

#[tokio::test]
async fn client_query_can_override_session_id_per_message() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    client
        .query_with_session_id("hi", "custom-session")
        .await
        .expect("query");

    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter(|write| !String::from_utf8_lossy(write).trim().is_empty())
        .map(|write| serde_json::from_slice(write).unwrap())
        .collect();
    assert_eq!(writes[1]["type"], "user");
    assert_eq!(writes[1]["message"]["content"], "hi");
    assert_eq!(writes[1]["session_id"], "custom-session");
}

#[tokio::test]
async fn client_query_stream_sends_prompt_frames_and_defaults_session_id() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    client
        .query_stream(futures::stream::iter([
            serde_json::json!({
                "type": "user",
                "message": {"role": "user", "content": "first"}
            }),
            serde_json::json!({
                "type": "user",
                "message": {"role": "user", "content": "second"},
                "session_id": "custom"
            }),
        ]))
        .await
        .expect("query stream");

    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter(|write| !String::from_utf8_lossy(write).trim().is_empty())
        .map(|write| serde_json::from_slice(write).unwrap())
        .collect();
    assert_eq!(writes[1]["message"]["content"], "first");
    assert_eq!(writes[1]["session_id"], "default");
    assert_eq!(writes[2]["message"]["content"], "second");
    assert_eq!(writes[2]["session_id"], "custom");
}

#[tokio::test]
async fn client_query_requires_explicit_connect_like_python_sdk() {
    let mut client = ClaudeAgentClient::with_transport(
        ClaudeAgentOptions::default(),
        Box::new(MockTransport::default()),
    )
    .expect("client");

    let err = client.query("hi").await.expect_err("not connected");
    assert!(err.to_string().contains("Not connected"));
}

#[tokio::test]
async fn client_receive_messages_drains_without_stopping_at_result() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "first"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "after result"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
    }

    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");
    client.connect().await.expect("connect");

    let messages = client.receive_messages().await.expect("messages");

    assert_eq!(messages.len(), 3);
    assert!(matches!(messages[0], Message::AssistantMsg { .. }));
    assert!(matches!(messages[1], Message::ResultMsg { .. }));
    assert!(matches!(messages[2], Message::AssistantMsg { .. }));
}

#[tokio::test]
async fn client_connect_with_prompt_sends_initial_user_message() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect_with_prompt("hello").await.expect("connect");

    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    assert_eq!(writes[0]["type"], "control_request");
    assert_eq!(writes[0]["request"]["subtype"], "initialize");
    assert_eq!(writes[1]["type"], "user");
    assert_eq!(writes[1]["message"]["content"], "hello");
    assert_eq!(writes[1]["session_id"], "default");
}

#[tokio::test]
async fn client_disconnect_is_python_compatible_abort_alias() {
    let transport = MockTransport::default();
    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn dynamic_control_methods_send_python_compatible_requests() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    state.lock().unwrap().auto_control_responses = true;
    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    client
        .set_permission_mode(claude_code_sdk_rust::PermissionMode::AcceptEdits)
        .await
        .expect("set permission mode");
    client
        .set_model(Some("claude-haiku-4-5-20251001".to_string()))
        .await
        .expect("set model");
    client.set_model(None).await.expect("clear model");
    client.interrupt().await.expect("interrupt");

    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    assert_eq!(writes[1]["request"]["subtype"], "set_permission_mode");
    assert_eq!(writes[1]["request"]["mode"], "acceptEdits");
    assert_eq!(writes[2]["request"]["subtype"], "set_model");
    assert_eq!(writes[2]["request"]["model"], "claude-haiku-4-5-20251001");
    assert_eq!(writes[3]["request"]["subtype"], "set_model");
    assert!(writes[3]["request"]["model"].is_null());
    assert_eq!(writes[4]["request"]["subtype"], "interrupt");
}

#[tokio::test]
async fn client_receive_response_answers_can_use_tool_control_requests() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "control_request",
                "request_id": "permission_1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "input": {"command": "pwd"},
                    "permission_suggestions": [],
                    "tool_use_id": "toolu_1"
                }
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "ok"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "ok"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default"
            }))
            .unwrap(),
        );
    }

    let options = ClaudeAgentOptions::builder()
        .can_use_tool(|tool_name, input, context| async move {
            assert_eq!(tool_name, "Bash");
            assert_eq!(
                input.get("command").and_then(|value| value.as_str()),
                Some("pwd")
            );
            assert_eq!(context.tool_use_id.as_deref(), Some("toolu_1"));
            Ok(claude_code_sdk_rust::PermissionResult::allow())
        })
        .build();
    let mut client = ClaudeAgentClient::with_transport(options, Box::new(transport)).unwrap();

    client.connect().await.unwrap();
    client.query("hi").await.unwrap();
    let messages = client.receive_response().await.unwrap();

    assert!(messages
        .iter()
        .any(|message| matches!(message, Message::ResultMsg { .. })));
    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    let permission_response = writes
        .iter()
        .find(|write| {
            write["type"] == "control_response" && write["response"]["request_id"] == "permission_1"
        })
        .expect("permission response");
    assert_eq!(
        permission_response["response"]["response"]["behavior"],
        "allow"
    );
}

#[tokio::test]
async fn client_stream_message_yields_events_from_response() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [
                        {"type": "text", "text": "hello"},
                        {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "Cargo.toml"}}
                    ]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default",
                "stop_reason": "end_turn"
            }))
            .unwrap(),
        );
    }

    let mut client =
        ClaudeAgentClient::with_transport(ClaudeAgentOptions::default(), Box::new(transport))
            .expect("client");

    client.connect().await.expect("connect");
    let mut events = client.stream_message("hi").await.expect("stream");
    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }

    assert!(matches!(
        collected.first(),
        Some(StreamEvent::ContentChunk(text)) if text == "hello"
    ));
    assert!(collected.iter().any(|event| matches!(
        event,
        StreamEvent::ToolUseStart { id, name, .. } if id == "toolu_1" && name == "Read"
    )));
    assert!(collected
        .iter()
        .any(|event| matches!(event, StreamEvent::Complete(_))));
}

#[tokio::test]
async fn client_stream_message_answers_can_use_tool_control_requests() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "control_request",
                "request_id": "permission_stream",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Read",
                    "input": {"file_path": "Cargo.toml"},
                    "permission_suggestions": [],
                    "tool_use_id": "toolu_stream"
                }
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-4-5",
                    "content": [{"type": "text", "text": "ok"}]
                },
                "session_id": "default"
            }))
            .unwrap(),
        );
        state.scripted_messages.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "duration_ms": 1,
                "duration_api_ms": 1,
                "is_error": false,
                "num_turns": 1,
                "session_id": "default"
            }))
            .unwrap(),
        );
    }

    let options = ClaudeAgentOptions::builder()
        .can_use_tool(|tool_name, input, context| async move {
            assert_eq!(tool_name, "Read");
            assert_eq!(
                input.get("file_path").and_then(|value| value.as_str()),
                Some("Cargo.toml")
            );
            assert_eq!(context.tool_use_id.as_deref(), Some("toolu_stream"));
            Ok(claude_code_sdk_rust::PermissionResult::allow())
        })
        .build();
    let mut client = ClaudeAgentClient::with_transport(options, Box::new(transport)).unwrap();

    client.connect().await.expect("connect");
    let mut events = client.stream_message("hi").await.expect("stream");
    let mut collected = Vec::new();
    while let Ok(event) = events.try_recv() {
        collected.push(event);
    }
    assert!(collected
        .iter()
        .any(|event| matches!(event, StreamEvent::Complete(_))));

    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    let permission_response = writes
        .iter()
        .find(|write| {
            write["type"] == "control_response"
                && write["response"]["request_id"] == "permission_stream"
        })
        .expect("permission response");
    assert_eq!(
        permission_response["response"]["response"]["behavior"],
        "allow"
    );
}
