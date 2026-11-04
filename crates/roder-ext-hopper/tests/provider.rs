use futures::StreamExt;
use roder_api::catalog::PROVIDER_HOPPER;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::{
    InputImage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
};
use roder_ext_hopper::{HopperConfig, HopperExtension, HopperInferenceEngine};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod support;
use support::TestEnv;

#[test]
fn installs_hopper_engine_with_offline_metadata() {
    let _env = TestEnv::new();
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(HopperExtension::new(HopperConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry
        .inference_engine(PROVIDER_HOPPER)
        .expect("hopper engine registered");

    let metadata = engine.metadata();
    assert_eq!(metadata.name, "Hopper");
    assert_eq!(metadata.auth_label.as_deref(), Some("HOPPER_API_KEY"));
    assert_eq!(metadata.auth_configured, Some(false));
}

#[tokio::test]
async fn list_models_returns_built_in_fallback_without_credentials() {
    let _env = TestEnv::new_async().await;
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(HopperExtension::new(HopperConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry.inference_engine(PROVIDER_HOPPER).unwrap();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_HOPPER,
        })
        .await
        .unwrap();
    assert!(
        models
            .iter()
            .any(|model| model.id == "deepseek-v4.1-flash-1017")
    );
    assert!(
        models
            .iter()
            .any(|model| model.id == "deepseek-v4.1-flash-1017-abliterated")
    );
}

#[tokio::test]
async fn stream_turn_fails_locally_without_api_key() {
    let _env = TestEnv::new_async().await;
    let engine = HopperInferenceEngine::new(HopperConfig::default());
    let error = match engine
        .stream_turn(turn_context(), text_request("deepseek-v4.1-flash-1017"))
        .await
    {
        Ok(_) => panic!("expected missing-key error"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("Hopper API key is missing"),
        "error: {error}"
    );
    assert!(error.contains("HOPPER_API_KEY"), "error: {error}");
    assert!(error.contains("[providers.hopper]"), "guidance: {error}");
}

#[tokio::test]
async fn stream_turn_sends_bearer_auth_and_model_id() {
    let _env = TestEnv::new_async().await;
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut stream = engine
        .stream_turn(turn_context(), text_request("deepseek-v4.1-flash-1017"))
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    let (headers, body) = server.request.await.unwrap();

    assert!(
        headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("authorization: Bearer sk_hopper_secret")),
        "headers: {headers:?}"
    );
    assert_eq!(body["model"], "deepseek-v4.1-flash-1017");
    assert_eq!(body["stream"], true);
}

#[tokio::test]
async fn stream_turn_sends_user_and_tool_result_images() {
    let _env = TestEnv::new_async().await;
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(server.base_url.clone()),
    });
    assert!(engine.capabilities().image_input);
    assert!(!engine.capabilities().structured_output);
    assert!(engine.tool_result_image_input("deepseek-v4.1-flash-1017"));
    assert!(engine.tool_result_image_input("deepseek-v4.1-flash-1017-abliterated"));
    assert!(!engine.tool_result_image_input("unknown-model"));
    let image_url = "https://images.example/photo.png";
    // Also exercises request capture with an inline image larger than 16 KiB.
    let inline_url = format!("data:image/png;base64,{}", "AAAA".repeat(8192));
    let mut request = text_request("deepseek-v4.1-flash-1017");
    request.transcript = vec![
        TranscriptItem::UserMessage(UserMessage::with_images(
            "Describe these images",
            vec![
                InputImage {
                    image_url: image_url.into(),
                },
                InputImage {
                    image_url: inline_url.clone(),
                },
            ],
        )),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "image-call".into(),
            name: "view_image".into(),
            arguments: "{}".into(),
        }),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "text-call".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "image-call".into(),
            name: Some("view_image".into()),
            result: "image loaded".into(),
            display_payload: Some(json!({
                "__view_image": { "image_url": "data:image/png;base64,YWJj" }
            })),
            is_error: false,
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "text-call".into(),
            name: Some("read_file".into()),
            result: "file contents".into(),
            display_payload: None,
            is_error: false,
        }),
    ];
    let mut stream = engine.stream_turn(turn_context(), request).await.unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    let (_, body) = server.request.await.unwrap();
    let messages = body["messages"].as_array().unwrap();
    let roles = messages
        .iter()
        .map(|message| message["role"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["user", "assistant", "tool", "tool", "user"]);
    assert_eq!(
        messages[0]["content"][0],
        json!({
            "type": "text", "text": "Describe these images"
        })
    );
    assert_eq!(
        messages[0]["content"][1],
        json!({
            "type": "image_url", "image_url": { "url": image_url }
        })
    );
    assert_eq!(
        messages[0]["content"][2],
        json!({
            "type": "image_url", "image_url": { "url": inline_url }
        })
    );
    assert_eq!(messages[2]["tool_call_id"], "image-call");
    assert_eq!(messages[3]["tool_call_id"], "text-call");
    assert!(
        messages[4]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| {
                part["type"] == "image_url"
                    && part["image_url"]["url"] == "data:image/png;base64,YWJj"
            })
    );
}

#[tokio::test]
async fn stream_turn_enables_thinking_mode_with_effort() {
    let _env = TestEnv::new_async().await;
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"reasoning_content\":\"cot\",\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut request = text_request("deepseek-v4.1-flash-1017");
    request.reasoning = ReasoningConfig {
        enabled: true,
        level: Some("high".to_string()),
    };
    let mut stream = engine.stream_turn(turn_context(), request).await.unwrap();
    while stream.next().await.is_some() {}
    let (_, body) = server.request.await.unwrap();

    assert_eq!(body["model"], "deepseek-v4.1-flash-1017");
    assert_eq!(body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(body["reasoning_effort"], "high");
}

#[tokio::test]
async fn stream_turn_disables_thinking_when_reasoning_off() {
    let _env = TestEnv::new_async().await;
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut stream = engine
        .stream_turn(turn_context(), text_request("deepseek-v4.1-flash-1017"))
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    let (_, body) = server.request.await.unwrap();

    assert_eq!(body["thinking"], json!({ "type": "disabled" }));
    assert!(body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn stream_turn_preserves_dotted_model_id() {
    let _env = TestEnv::new_async().await;
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut stream = engine
        .stream_turn(
            turn_context(),
            text_request("deepseek-v4.1-flash-1017-abliterated"),
        )
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    let (_, body) = server.request.await.unwrap();

    assert_eq!(body["model"], "deepseek-v4.1-flash-1017-abliterated");
}

#[tokio::test]
async fn stream_turn_error_body_includes_response_but_scrubs_key() {
    let _env = TestEnv::new_async().await;
    let base_url = spawn_error_server(
        "HTTP/1.1 401 Unauthorized",
        "{\"error\":\"bad key sk_hopper_secret should not appear\"}",
    )
    .await;
    let engine = HopperInferenceEngine::new(HopperConfig {
        api_key: Some("sk_hopper_secret".to_string()),
        base_url: Some(base_url),
    });

    let error = match engine
        .stream_turn(turn_context(), text_request("deepseek-v4.1-flash-1017"))
        .await
    {
        Ok(_) => panic!("expected provider error"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("401 Unauthorized"), "error: {error}");
    assert!(error.contains("authentication or permission failed"));
    assert!(!error.contains("sk_hopper_secret"), "leaked key: {error}");
    assert!(
        error.contains("bad key") || error.contains("<redacted>"),
        "body should be included: {error}"
    );
}

fn turn_context<'a>() -> InferenceTurnContext<'a> {
    InferenceTurnContext {
        thread_id: "hopper-test",
        turn_id: "turn-hopper-test",
        tool_executor: None,
    }
}

fn text_request(model: &str) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_HOPPER.to_string(),
            model: model.to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
        tools: Vec::<ToolSpec>::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(32),
            ..OutputConfig::default()
        },
        runtime: RuntimeHints::default(),
        metadata: json!({}),
    }
}

struct CapturedChatServer {
    base_url: String,
    request: tokio::sync::oneshot::Receiver<(Vec<String>, Value)>,
}

async fn spawn_chat_server(
    expected_path: &'static str,
    response_body: &'static str,
) -> CapturedChatServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (request, headers, body) = read_http_request(&mut stream).await;
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");
        assert_eq!(path, expected_path);
        tx.send((headers, serde_json::from_slice(&body).unwrap()))
            .unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    CapturedChatServer {
        base_url: format!("http://{addr}"),
        request: rx,
    }
}

async fn spawn_error_server(status_line: &'static str, response_body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut stream).await;
        let response = format!(
            "{status_line}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    format!("http://{addr}")
}

async fn read_http_request(
    stream: &mut (impl tokio::io::AsyncRead + Unpin),
) -> (String, Vec<String>, Vec<u8>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        let mut chunk = [0; 1024];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "connection closed before request headers");
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let headers = head
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let content_length = headers
        .iter()
        .find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .expect("request has Content-Length");
    let expected_end = header_end + content_length;
    if bytes.len() < expected_end {
        let received = bytes.len();
        bytes.resize(expected_end, 0);
        stream.read_exact(&mut bytes[received..]).await.unwrap();
    }
    (head, headers, bytes[header_end..expected_end].to_vec())
}

#[tokio::test]
async fn captures_segmented_headers_and_large_request_body() {
    let body = serde_json::to_vec(&json!({ "prompt": "x".repeat(40 * 1024) })).unwrap();
    let expected_body = body.clone();
    let (mut sender, mut receiver) = tokio::io::duplex(31);
    let writer = tokio::spawn(async move {
        let head = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nCoNtEnT-LeNgTh: {}\r\n\r\n",
            body.len()
        );
        // The small duplex buffer forces header and body segmentation.
        for chunk in head.as_bytes().chunks(7) {
            sender.write_all(chunk).await.unwrap();
        }
        for chunk in body.chunks(127) {
            sender.write_all(chunk).await.unwrap();
        }
    });
    let (head, _, actual_body) = read_http_request(&mut receiver).await;
    assert!(head.starts_with("POST /v1/chat/completions"));
    assert_eq!(actual_body, expected_body);
    writer.await.unwrap();
}
