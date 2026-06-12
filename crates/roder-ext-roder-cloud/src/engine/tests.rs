use std::sync::{Arc, Mutex};

use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InferenceProviderContext, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

struct RecordedRequest {
    path: String,
    authorization: String,
    body: String,
}

/**
 * Sequential fake HTTP server: serves one scripted `(status, body)` response
 * per accepted connection and records each request's path, Authorization
 * header, and body. Hosts both the Rails token-exchange path and the
 * inference edge paths in tests.
 */
async fn spawn_fake_server(
    responses: Vec<(u16, String)>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut raw = Vec::new();
            let mut buf = [0_u8; 4096];
            loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some((head, tail)) = text.split_once("\r\n\r\n") {
                    let content_length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if tail.len() >= content_length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&raw).to_string();
            let (head, body_text) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
            let path = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let authorization = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("authorization")
                        .then(|| value.trim().to_string())
                })
                .unwrap_or_default();
            requests.lock().unwrap().push(RecordedRequest {
                path,
                authorization,
                body: body_text.to_string(),
            });
            let status_text = match status {
                200 => "OK",
                401 => "Unauthorized",
                403 => "Forbidden",
                429 => "Too Many Requests",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}")
}

fn token_body(token: &str, expires_in: u64) -> String {
    json!({ "token": token, "token_type": "Bearer", "expires_in": expires_in }).to_string()
}

fn completed_body(text: &str) -> String {
    json!({
        "created_at": 1_718_000_000,
        "status": "completed",
        "id": "resp_test",
        "object": "response",
        "model": "roder.cloud/free",
        "output": [{
            "id": "resp_test_message",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text }]
        }],
        "output_text": text,
        "usage": { "input_tokens": 7, "output_tokens": 5, "total_tokens": 12 }
    })
    .to_string()
}

fn error_body(code: &str, message: &str) -> String {
    json!({ "error": { "code": code, "message": message } }).to_string()
}

fn turn_request() -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "roder-cloud".to_string(),
            model: "roder.cloud/free".to_string(),
        },
        instructions: InstructionBundle {
            system: Some("be brief".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hello"))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: Value::Null,
    }
}

fn turn_ctx<'a>() -> InferenceTurnContext<'a> {
    InferenceTurnContext {
        thread_id: "thread-1",
        turn_id: "turn-1",
        tool_executor: None,
    }
}

fn engine_for(base: &str) -> RoderCloudEngine {
    RoderCloudEngine::new(
        Some("roder_test_key".to_string()),
        Some(format!("{base}/v1")),
        Some(base.to_string()),
    )
}

async fn collect_events(stream: InferenceEventStream) -> Vec<InferenceEvent> {
    use futures::StreamExt;
    stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn turn_exchanges_token_and_synthesizes_stream_events() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_fake_server(
        vec![
            (200, token_body("jwt-1", 600)),
            (200, completed_body("hi there")),
        ],
        Arc::clone(&requests),
    )
    .await;
    let engine = engine_for(&base);

    let events = collect_events(
        engine
            .stream_turn(turn_ctx(), turn_request())
            .await
            .unwrap(),
    )
    .await;

    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].path, "/api/v1/inference_tokens");
    assert_eq!(recorded[0].authorization, "Bearer roder_test_key");
    assert_eq!(recorded[1].path, "/v1/responses");
    assert_eq!(recorded[1].authorization, "Bearer jwt-1");
    let body: Value = serde_json::from_str(&recorded[1].body).unwrap();
    assert_eq!(body["model"], "roder.cloud/free");
    assert_eq!(body["stream"], false);
    assert_eq!(body["instructions"], "be brief");
    assert_eq!(body["max_output_tokens"], 128);
    assert!(body.get("background").is_none());
    assert!(body.get("tools").is_none());
    assert!(body.get("store").is_none());
    assert_eq!(body["input"], "hello");

    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0],
        InferenceEvent::MessageDelta(MessageDelta {
            text: "hi there".to_string(),
            phase: None,
        })
    );
    let InferenceEvent::Usage(usage) = &events[1] else {
        panic!("expected usage event, got {:?}", events[1]);
    };
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 12);
    assert_eq!(
        events[2],
        InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some("stop".to_string()),
            provider_response_id: Some("resp_test".to_string()),
        })
    );
}

#[tokio::test]
async fn cached_token_is_reused_across_turns() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_fake_server(
        vec![
            (200, token_body("jwt-1", 600)),
            (200, completed_body("one")),
            (200, completed_body("two")),
        ],
        Arc::clone(&requests),
    )
    .await;
    let engine = engine_for(&base);

    for _ in 0..2 {
        collect_events(
            engine
                .stream_turn(turn_ctx(), turn_request())
                .await
                .unwrap(),
        )
        .await;
    }

    let recorded = requests.lock().unwrap();
    let token_calls = recorded
        .iter()
        .filter(|request| request.path == "/api/v1/inference_tokens")
        .count();
    assert_eq!(token_calls, 1, "second turn must reuse the cached JWT");
    assert_eq!(recorded.len(), 3);
}

#[tokio::test]
async fn invalid_token_triggers_one_refresh_and_retry() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_fake_server(
        vec![
            (200, token_body("jwt-stale", 600)),
            (401, error_body("invalid_token", "token expired")),
            (200, token_body("jwt-fresh", 600)),
            (200, completed_body("recovered")),
        ],
        Arc::clone(&requests),
    )
    .await;
    let engine = engine_for(&base);

    let events = collect_events(
        engine
            .stream_turn(turn_ctx(), turn_request())
            .await
            .unwrap(),
    )
    .await;

    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 4);
    assert_eq!(recorded[3].authorization, "Bearer jwt-fresh");
    assert!(matches!(events[0], InferenceEvent::MessageDelta(_)));
}

#[tokio::test]
async fn quota_and_model_errors_surface_actionable_messages() {
    for (status, code, expectation) in [
        (429_u16, "quota_exceeded", "quota exceeded"),
        (403_u16, "model_not_allowed", "enable it for your team"),
    ] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let base = spawn_fake_server(
            vec![
                (200, token_body("jwt-1", 600)),
                (status, error_body(code, "denied")),
            ],
            Arc::clone(&requests),
        )
        .await;
        let engine = engine_for(&base);

        let error = engine
            .stream_turn(turn_ctx(), turn_request())
            .await
            .err()
            .expect("turn must fail");
        let message = error.to_string();
        assert!(
            message.contains(expectation),
            "{code}: unexpected message {message:?}"
        );
    }
}

#[tokio::test]
async fn invalid_api_key_names_the_config_surface() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_fake_server(
        vec![(401, json!({ "error": "invalid_api_key" }).to_string())],
        Arc::clone(&requests),
    )
    .await;
    let engine = engine_for(&base);

    let error = engine
        .stream_turn(turn_ctx(), turn_request())
        .await
        .err()
        .expect("turn must fail");
    let message = error.to_string();
    assert!(message.contains("invalid_api_key"), "{message:?}");
    assert!(message.contains("RODER_CLOUD_API_KEY"), "{message:?}");
}

#[tokio::test]
async fn missing_api_key_fails_with_guidance() {
    let engine = RoderCloudEngine::new(None, Some("http://127.0.0.1:1/v1".to_string()), None);

    let error = engine
        .stream_turn(turn_ctx(), turn_request())
        .await
        .err()
        .expect("turn must fail");
    assert!(error.to_string().contains("RODER_CLOUD_API_KEY"));
}

#[tokio::test]
async fn missing_base_url_fails_with_guidance() {
    let engine = RoderCloudEngine::new(Some("roder_key".to_string()), None, None);

    let error = engine
        .stream_turn(turn_ctx(), turn_request())
        .await
        .err()
        .expect("turn must fail");
    assert!(error.to_string().contains("RODER_CLOUD_BASE_URL"));
}

#[tokio::test]
async fn list_models_uses_edge_and_keeps_catalog_metadata() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_fake_server(
        vec![
            (200, token_body("jwt-1", 600)),
            (
                200,
                json!({
                    "object": "list",
                    "data": [
                        { "id": "roder.cloud/free", "object": "model", "owned_by": "openrouter" },
                        { "id": "custom/team-model", "object": "model", "owned_by": "fireworks" }
                    ]
                })
                .to_string(),
            ),
        ],
        Arc::clone(&requests),
    )
    .await;
    let engine = engine_for(&base);

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_RODER_CLOUD,
        })
        .await
        .unwrap();

    assert_eq!(requests.lock().unwrap()[1].path, "/v1/models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "roder.cloud/free");
    assert!(
        models[0].context_window.is_some(),
        "catalog metadata must be preserved for known ids"
    );
    assert_eq!(models[1].id, "custom/team-model");
    assert_eq!(models[1].context_window, None);
}

#[tokio::test]
async fn list_models_without_key_falls_back_to_catalog() {
    let engine = RoderCloudEngine::new(None, None, None);

    let models = engine
        .list_models(InferenceProviderContext {
            provider_id: PROVIDER_RODER_CLOUD,
        })
        .await
        .unwrap();

    assert_eq!(models.len(), 4);
    assert_eq!(models[0].id, "roder.cloud/free");
}

#[test]
fn map_request_prunes_unsupported_fields() {
    let mut request = turn_request();
    request.runtime.prompt_cache_key = Some("cache-key".to_string());
    request.runtime.auto_compact_token_limit = Some(1000);
    request.reasoning = ReasoningConfig {
        enabled: true,
        level: Some("high".to_string()),
    };

    let body = RoderCloudEngine::map_request(&request);

    assert_eq!(body["stream"], false);
    assert_eq!(body["model"], "roder.cloud/free");
    assert!(body.get("prompt_cache_key").is_none());
    assert!(body.get("context_management").is_none());
    assert!(body.get("reasoning").is_none());
    assert!(body.get("include").is_none());
    assert!(body.get("background").is_none());
}

#[test]
fn map_request_flattens_multi_turn_transcripts() {
    use roder_api::transcript::AssistantMessage;
    let mut request = turn_request();
    request.transcript = vec![
        TranscriptItem::UserMessage(UserMessage::text("first")),
        TranscriptItem::AssistantMessage(AssistantMessage {
            text: "reply".to_string(),
            phase: None,
        }),
        TranscriptItem::UserMessage(UserMessage::text("second")),
    ];

    let body = RoderCloudEngine::map_request(&request);

    assert_eq!(body["input"], "user: first\nassistant: reply\nuser: second");
}

#[test]
fn metadata_reports_auth_state() {
    let configured = RoderCloudEngine::new(Some("roder_key".to_string()), None, None);
    assert_eq!(configured.metadata().auth_configured, Some(true));
    assert_eq!(configured.metadata().auth_type, ProviderAuthType::ApiKey);

    let unconfigured = RoderCloudEngine::new(None, None, None);
    assert_eq!(unconfigured.metadata().auth_configured, Some(false));
}
