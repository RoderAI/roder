use async_trait::async_trait;
use claude_code_sdk_rust::internal::transport::Transport;
use claude_code_sdk_rust::{
    create_sdk_mcp_server, query_messages_with_transport, query_stream_messages_with_transport,
    tool, ClaudeAgentOptions, Message, Result,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MockState {
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
    initialized: bool,
    input_closed: bool,
    closed: bool,
}

#[derive(Clone, Default)]
struct MockTransport {
    state: Arc<Mutex<MockState>>,
}

#[async_trait]
impl Transport for MockTransport {
    async fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.state.lock().unwrap().writes.push(data.to_vec());
        Ok(())
    }

    async fn close_input(&mut self) -> Result<()> {
        self.state.lock().unwrap().input_closed = true;
        Ok(())
    }

    async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        if !state.initialized {
            let first_write = state.writes.first().expect("initialize write");
            let initialize: serde_json::Value = serde_json::from_slice(first_write)?;
            let request_id = initialize["request_id"].as_str().unwrap();
            state.initialized = true;
            return Ok(Some(serde_json::to_vec(&serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {}
                }
            }))?));
        }
        Ok(state.reads.pop_front())
    }

    async fn close(&mut self) -> Result<()> {
        self.state.lock().unwrap().closed = true;
        Ok(())
    }
}

#[tokio::test]
async fn query_messages_with_transport_answers_post_prompt_sdk_mcp_control_requests() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.reads.push_back(
            serde_json::to_vec(&serde_json::json!({
                "type": "control_request",
                "request_id": "mcp_1",
                "request": {
                    "subtype": "mcp_message",
                    "server_name": "people",
                    "message": {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "tools/call",
                        "params": {"name": "greet", "arguments": {"name": "Ada"}}
                    }
                }
            }))
            .unwrap(),
        );
        state.reads.push_back(
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

    let server = create_sdk_mcp_server(
        "people",
        vec![tool(
            "greet",
            "Greet a user",
            serde_json::json!({"type": "object"}),
            |input| {
                let name = input
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("there");
                Ok(vec![claude_code_sdk_rust::MCPContent::Text {
                    text: format!("Hello, {name}!"),
                }])
            },
        )],
    );
    let options = ClaudeAgentOptions::builder()
        .sdk_mcp_server("people", server)
        .build();

    let messages = query_messages_with_transport("hi", Some(options), Box::new(transport))
        .await
        .expect("query");

    assert!(messages
        .iter()
        .any(|message| matches!(message, Message::ResultMsg { .. })));
    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    let mcp_response = writes
        .iter()
        .find(|write| {
            write["type"] == "control_response" && write["response"]["request_id"] == "mcp_1"
        })
        .expect("mcp response");
    assert_eq!(
        mcp_response["response"]["response"]["mcp_response"]["result"]["content"][0]["text"],
        "Hello, Ada!"
    );
    assert!(state.closed);
}

#[tokio::test]
async fn query_stream_messages_with_transport_writes_streamed_prompt_frames() {
    let transport = MockTransport::default();
    let state = transport.state.clone();
    {
        let mut state = state.lock().unwrap();
        state.reads.push_back(
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

    let stream = futures::stream::iter(vec![
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": "first"}
        }),
        serde_json::json!({
            "type": "user",
            "session_id": "custom",
            "message": {"role": "user", "content": "second"}
        }),
    ]);

    let messages = query_stream_messages_with_transport(stream, None, Box::new(transport))
        .await
        .expect("query");

    assert!(messages
        .iter()
        .any(|message| matches!(message, Message::ResultMsg { .. })));
    let state = state.lock().unwrap();
    let writes: Vec<serde_json::Value> = state
        .writes
        .iter()
        .filter_map(|write| serde_json::from_slice(write).ok())
        .collect();
    let user_writes: Vec<&serde_json::Value> = writes
        .iter()
        .filter(|write| write["type"] == "user")
        .collect();
    assert_eq!(user_writes.len(), 2);
    assert_eq!(user_writes[0]["session_id"], "default");
    assert_eq!(user_writes[1]["session_id"], "custom");
    assert!(state.input_closed);
    assert!(state.closed);
}
