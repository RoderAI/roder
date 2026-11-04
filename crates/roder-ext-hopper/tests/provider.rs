use futures::StreamExt;
use roder_api::catalog::PROVIDER_HOPPER;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_hopper::{HopperConfig, HopperExtension, HopperInferenceEngine};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Point `RODER_CONFIG_DIR` at an empty temp dir once per test binary so the
/// user's real `~/.roder/config.toml` (which may contain a hopper key)
/// cannot leak into tests that assert the unauthenticated state.
static CONFIG_ISOLATION: OnceLock<()> = OnceLock::new();

fn isolate_config_dir() {
    CONFIG_ISOLATION.get_or_init(|| {
        let temp =
            std::env::temp_dir().join(format!("roder-ext-hopper-tests-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        // SAFETY: set once before any test reads the config; all tests run in
        // the same process and never restore a real config dir.
        unsafe {
            std::env::set_var("RODER_CONFIG_DIR", &temp);
        }
    });
}

#[test]
fn installs_hopper_engine_with_offline_metadata() {
    isolate_config_dir();
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
    isolate_config_dir();
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
async fn stream_turn_enables_thinking_mode_with_effort() {
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
        let mut buf = vec![0_u8; 16 * 1024];
        let n = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");
        assert_eq!(path, expected_path);
        let headers = request
            .lines()
            .skip(1)
            .take_while(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        tx.send((headers, serde_json::from_str(body).unwrap()))
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
        let mut buf = vec![0_u8; 16 * 1024];
        let _ = stream.read(&mut buf).await.unwrap();
        let response = format!(
            "{status_line}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    format!("http://{addr}")
}
