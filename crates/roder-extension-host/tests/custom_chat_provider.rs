use std::time::Duration;

use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceProviderContext, InferenceTurnContext,
    ModelSelection,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::{
    AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
};
use roder_config::CustomProviderProtocol;
use roder_extension_host::{
    CustomInferenceProviderConfig, DefaultRegistryConfig, build_default_registry,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const PROVIDER: &str = "test-native-chat";
const MODEL: &str = "test/model";
const API_KEY: &str = "local-test-key";

struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Value,
}

struct MockServer {
    base_url: String,
    requests: JoinHandle<Vec<CapturedRequest>>,
}

async fn mock_server(responses: Vec<(&'static str, String)>) -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = tokio::spawn(async move {
        let mut captured = Vec::new();
        for (content_type, response_body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (header_end, content_length) = loop {
                let mut buffer = [0; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request ended before its headers");
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < header_end + content_length {
                let mut buffer = [0; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request ended before its body");
                bytes.extend_from_slice(&buffer[..read]);
            }
            let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
            captured.push(CapturedRequest {
                request_line: headers.lines().next().unwrap().to_string(),
                headers: headers
                    .lines()
                    .skip(1)
                    .filter_map(|line| line.split_once(':'))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
                    .collect(),
                body: if content_length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
                },
            });
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        captured
    });
    MockServer {
        base_url: format!("http://{address}/v1/"),
        requests,
    }
}

fn sse(frames: &[Value]) -> String {
    let mut body = frames
        .iter()
        .map(|frame| format!("data: {frame}\r\n\r\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\r\n\r\n");
    body
}

fn provider(base_url: &str, models: Vec<String>) -> CustomInferenceProviderConfig {
    CustomInferenceProviderConfig {
        id: PROVIDER.into(),
        name: Some("Native chat test".into()),
        api_key: Some(API_KEY.into()),
        base_url: base_url.into(),
        protocol: CustomProviderProtocol::ChatCompletions,
        models,
        context_window: Some(16_384),
        tool_allowlist: vec!["echo".into()],
        capabilities: Default::default(),
    }
}

fn request(transcript: Vec<TranscriptItem>) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER.into(),
            model: MODEL.into(),
        },
        instructions: Default::default(),
        transcript,
        tools: ["echo", "shell"]
            .into_iter()
            .map(|name| ToolSpec {
                name: name.into(),
                description: format!("Run {name}"),
                parameters: json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
            })
            .collect(),
        tool_choice: ToolChoice::Auto,
        reasoning: Default::default(),
        output: Default::default(),
        runtime: Default::default(),
        metadata: json!({}),
    }
}

fn turn_context() -> InferenceTurnContext<'static> {
    InferenceTurnContext {
        thread_id: "test-thread",
        turn_id: "test-turn",
        tool_executor: None,
    }
}

#[tokio::test]
async fn native_custom_chat_streams_filtered_tools_and_replays_tool_result() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let server = mock_server(vec![
            (
                "text/event-stream",
                sse(&[
                    json!({"id":"chat-first","choices":[{"delta":{"content":"Checking."},"finish_reason":null}]}),
                    json!({"id":"chat-first","choices":[{"delta":{"tool_calls":[{
                        "index":0,"id":"echo-1","type":"function",
                        "function":{"name":"echo","arguments":"{\"text\":"}
                    }]},"finish_reason":null}]}),
                    json!({"id":"chat-first","choices":[{"delta":{"tool_calls":[{
                        "index":0,"function":{"arguments":"\"ping\"}"}
                    }]},"finish_reason":"tool_calls"}]}),
                ]),
            ),
            (
                "text/event-stream",
                sse(&[
                    json!({"id":"chat-second","choices":[{"delta":{"content":"pong"},"finish_reason":null}]}),
                    json!({"id":"chat-second","choices":[{"delta":{},"finish_reason":"stop"}]}),
                ]),
            ),
        ])
        .await;
        let root = tempfile::tempdir().unwrap();
        let registry = build_default_registry(DefaultRegistryConfig {
            custom_inference_providers: vec![provider(&server.base_url, vec![MODEL.into()])],
            workspace: Some(root.path().into()),
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        let engine = registry.inference_engine(PROVIDER).unwrap();
        assert!(engine.capabilities().streaming);
        assert!(engine.capabilities().tool_calls);
        assert!(!engine.capabilities().image_input);
        let models = engine
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER,
            })
            .await
            .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, MODEL);
        assert_eq!(models[0].context_window, Some(16_384));

        let mut transcript = vec![TranscriptItem::UserMessage(UserMessage::text(
            "Echo ping, then say pong.",
        ))];
        let mut first = engine
            .stream_turn(turn_context(), request(transcript.clone()))
            .await
            .unwrap();
        let mut text = String::new();
        let mut tool_call = None;
        let mut completed = false;
        while let Some(event) = first.next().await {
            match event.unwrap() {
                InferenceEvent::MessageDelta(delta) => text.push_str(&delta.text),
                InferenceEvent::ToolCallCompleted(call) => {
                    assert!(tool_call.replace(call).is_none());
                }
                InferenceEvent::Completed(_) => completed = true,
                InferenceEvent::Failed(failure) => panic!("unexpected failure: {failure:?}"),
                _ => {}
            }
        }
        assert!(completed);
        assert_eq!(text, "Checking.");
        let call = tool_call.expect("native chat must emit a completed tool call");
        assert_eq!(call.id, "echo-1");
        assert_eq!(call.name, "echo");
        assert_eq!(
            serde_json::from_str::<Value>(&call.arguments).unwrap(),
            json!({"text":"ping"})
        );
        transcript.extend([
            TranscriptItem::AssistantMessage(AssistantMessage {
                text,
                phase: None,
            }),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments,
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: call.id,
                name: Some(call.name),
                result: "ping".into(),
                display_payload: None,
                is_error: false,
            }),
        ]);
        let mut follow_up = engine
            .stream_turn(turn_context(), request(transcript))
            .await
            .unwrap();
        let mut final_text = String::new();
        let mut completed = false;
        while let Some(event) = follow_up.next().await {
            match event.unwrap() {
                InferenceEvent::MessageDelta(delta) => final_text.push_str(&delta.text),
                InferenceEvent::Completed(_) => completed = true,
                InferenceEvent::Failed(failure) => panic!("unexpected failure: {failure:?}"),
                _ => {}
            }
        }
        assert!(completed);
        assert_eq!(final_text, "pong");

        let requests = server.requests.await.unwrap();
        assert_eq!(requests.len(), 2, "explicit models must not query /models");
        for request in &requests {
            assert_eq!(request.request_line, "POST /v1/chat/completions HTTP/1.1");
            assert!(request.headers.iter().any(|(name, value)| {
                name == "authorization" && value == &format!("Bearer {API_KEY}")
            }));
            assert_eq!(request.body["model"], MODEL);
            assert_eq!(request.body["stream"], true);
            assert_eq!(request.body["tools"].as_array().unwrap().len(), 1);
            assert_eq!(request.body["tools"][0]["function"]["name"], "echo");
        }
        let messages = requests[1].body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "Checking.");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "echo-1");
        assert_eq!(messages[1]["tool_calls"][0]["function"]["name"], "echo");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "echo-1");
        assert_eq!(messages[2]["content"], "ping");
    })
    .await
    .expect("mock chat round trip timed out");
}

#[tokio::test]
async fn native_custom_chat_discovers_models_with_context_window_override() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let server = mock_server(vec![(
            "application/json",
            json!({"data":[{"id":MODEL,"object":"model","context_length":32768}]}).to_string(),
        )])
        .await;
        let root = tempfile::tempdir().unwrap();
        let registry = build_default_registry(DefaultRegistryConfig {
            custom_inference_providers: vec![provider(&server.base_url, vec![])],
            workspace: Some(root.path().into()),
            data_dir: Some(root.path().join("data")),
            ..Default::default()
        })
        .unwrap();
        let models = registry
            .inference_engine(PROVIDER)
            .unwrap()
            .list_models(InferenceProviderContext {
                provider_id: PROVIDER,
            })
            .await
            .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, MODEL);
        assert_eq!(models[0].context_window, Some(16_384));
        let requests = server.requests.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].request_line, "GET /v1/models HTTP/1.1");
        assert!(requests[0].headers.iter().any(|(name, value)| {
            name == "authorization" && value == &format!("Bearer {API_KEY}")
        }));
    })
    .await
    .expect("mock model discovery timed out");
}
