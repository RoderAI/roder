use futures::StreamExt;
use roder_api::catalog::PROVIDER_SYNTHETIC;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::{ToolChoice, ToolSpec};
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_synthetic::{SyntheticConfig, SyntheticExtension, SyntheticInferenceEngine};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Point `RODER_CONFIG_DIR` at an empty temp dir once per test binary so the
/// user's real `~/.roder/config.toml` (which may contain a synthetic key)
/// cannot leak into tests that assert the unauthenticated state.
static CONFIG_ISOLATION: OnceLock<()> = OnceLock::new();

fn isolate_config_dir() {
    CONFIG_ISOLATION.get_or_init(|| {
        let temp = std::env::temp_dir().join(format!(
            "roder-ext-synthetic-tests-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        // SAFETY: set once before any test reads the config; all tests run in
        // the same process and never restore a real config dir.
        unsafe { std::env::set_var("RODER_CONFIG_DIR", &temp); }
    });
}

#[test]
fn installs_synthetic_engine_with_offline_metadata() {
    isolate_config_dir();
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(SyntheticExtension::new(SyntheticConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry
        .inference_engine(PROVIDER_SYNTHETIC)
        .expect("synthetic engine registered");

    let metadata = engine.metadata();
    assert_eq!(metadata.name, "Synthetic");
    assert_eq!(metadata.auth_label.as_deref(), Some("SYNTHETIC_API_KEY"));
    assert_eq!(metadata.auth_configured, Some(false));
}

#[tokio::test]
async fn list_models_returns_alias_fallback_without_credentials() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder
        .install(SyntheticExtension::new(SyntheticConfig::default()))
        .unwrap();
    let registry = builder.build().unwrap();
    let engine = registry.inference_engine(PROVIDER_SYNTHETIC).unwrap();

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_SYNTHETIC,
        })
        .await
        .unwrap();

    assert!(
        models.iter().any(|model| {
            model.id == "syn:large:text" && model.name == "Synthetic Large (Text)"
        })
    );
    assert!(models.iter().any(|model| model.id == "syn:large:vision"));
    assert!(models.iter().any(|model| model.id == "hf:zai-org/GLM-5.2"));
    assert!(models.iter().any(|model| model.id == "hf:MiniMaxAI/MiniMax-M3"));
    assert!(models.iter().any(|model| model.id == "hf:openai/gpt-oss-120b"));
}

#[tokio::test]
async fn stream_turn_without_credentials_fails_before_network() {
    isolate_config_dir();
    let engine = SyntheticInferenceEngine::new(SyntheticConfig {
        api_key: Some(String::new()),
        base_url: None,
    });
    let error = match engine
        .stream_turn(turn_context(), text_request("syn:large:text"))
        .await
    {
        Ok(_) => panic!("expected missing-key failure"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("SYNTHETIC_API_KEY"), "guidance: {error}");
    assert!(error.contains("[providers.synthetic]"), "guidance: {error}");
}

#[tokio::test]
async fn stream_turn_sends_documented_bearer_chat_request() {
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = SyntheticInferenceEngine::new(SyntheticConfig {
        api_key: Some("syn-secret-key".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut request = text_request("syn:large:text");
    request.tools = vec![ToolSpec {
        name: "run command".to_string(),
        description: "Run a command".to_string(),
        parameters: json!({ "type": "object", "properties": {} }),
    }];
    request.output.response_format = Some(json!({ "type": "json_object" }));

    let mut stream = engine.stream_turn(turn_context(), request).await.unwrap();
    while stream.next().await.is_some() {}
    let (headers, body) = server.request.await.unwrap();

    assert!(
        headers
            .iter()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer syn-secret-key")),
        "expected bearer auth header, got: {headers:?}"
    );
    assert_eq!(body["model"], "syn:large:text");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["tools"][0]["function"]["name"], "run_command");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["response_format"], json!({ "type": "json_object" }));
}

#[tokio::test]
async fn stream_turn_preserves_concrete_hf_model_id() {
    let server = spawn_chat_server(
        "/chat/completions",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n",
    )
    .await;
    let engine = SyntheticInferenceEngine::new(SyntheticConfig {
        api_key: Some("syn-secret-key".to_string()),
        base_url: Some(server.base_url.clone()),
    });

    let mut stream = engine
        .stream_turn(turn_context(), text_request("hf:zai-org/GLM-5.2"))
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    let (_, body) = server.request.await.unwrap();

    assert_eq!(body["model"], "hf:zai-org/GLM-5.2");
}

#[tokio::test]
async fn stream_turn_error_body_includes_response_but_scrubs_key() {
    let base_url = spawn_error_server(
        "HTTP/1.1 401 Unauthorized",
        "{\"error\":\"bad key syn-secret-key should not appear\"}",
    )
    .await;
    let engine = SyntheticInferenceEngine::new(SyntheticConfig {
        api_key: Some("syn-secret-key".to_string()),
        base_url: Some(base_url),
    });

    let error = match engine
        .stream_turn(turn_context(), text_request("syn:large:text"))
        .await
    {
        Ok(_) => panic!("expected provider error"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("401 Unauthorized"), "error: {error}");
    assert!(error.contains("authentication or permission failed"));
    // The auth credential must be scrubbed from the body.
    assert!(!error.contains("syn-secret-key"), "leaked key: {error}");
    // The response body is now included for the TUI popup.
    assert!(
        error.contains("bad key") || error.contains("<redacted>"),
        "body should be included: {error}"
    );
}

fn turn_context<'a>() -> InferenceTurnContext<'a> {
    InferenceTurnContext {
        thread_id: "synthetic-test",
        turn_id: "turn-synthetic-test",
        tool_executor: None,
    }
}

fn text_request(model: &str) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_SYNTHETIC.to_string(),
            model: model.to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
        tools: Vec::new(),
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
