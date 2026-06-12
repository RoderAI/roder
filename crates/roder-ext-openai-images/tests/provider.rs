//! Offline fake-HTTP coverage for the OpenAI GPT Image provider: exact
//! generation/edit request bodies, response and usage parsing, option
//! validation before network calls, retry classification, and redacted
//! provider error mapping (roadmap phase 91). No network or credentials.

use base64::Engine;
use roder_api::media::{
    ImageGenerationAction, MediaGenerationRequest, MediaGeneratorProvider, MediaImageInput,
};
use roder_api::reliability::ReliabilityRequestPolicy;
use roder_ext_openai_images::{OpenAiImagesConfig, OpenAiImagesProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Fake HTTP server answering scripted (status, body) responses in order and
/// capturing each raw request (headers + body).
async fn spawn_server(responses: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel(responses.len().max(1));
    tokio::spawn(async move {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0u8; 16384];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&raw);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if raw.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            let reason = if status < 400 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            let _ = stream.shutdown().await;
            let _ = tx.send(String::from_utf8_lossy(&raw).into_owned()).await;
        }
    });
    (base_url, rx)
}

fn provider_for(base_url: &str) -> OpenAiImagesProvider {
    OpenAiImagesProvider::new(
        OpenAiImagesConfig::new(Some("test-secret".to_string()))
            .with_base_url(base_url)
            .with_retry_policy(ReliabilityRequestPolicy {
                provider_retry_initial_backoff_ms: 0,
                ..ReliabilityRequestPolicy::default()
            }),
    )
}

fn request(prompt: &str) -> MediaGenerationRequest {
    MediaGenerationRequest {
        prompt: prompt.to_string(),
        ..MediaGenerationRequest::default()
    }
}

fn body_of(raw: &str) -> serde_json::Value {
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default();
    serde_json::from_str(body).unwrap()
}

const GENERATION_RESPONSE: &str = r#"{
    "created": 1713833628,
    "data": [
        {"b64_json": "aW1hZ2Ux", "revised_prompt": "a refined tiny image"},
        {"b64_json": "aW1hZ2Uy"}
    ],
    "output_format": "png",
    "size": "1024x1024",
    "usage": {
        "input_tokens": 10,
        "input_tokens_details": {"image_tokens": 0, "text_tokens": 10},
        "output_tokens": 1056,
        "total_tokens": 1066
    }
}"#;

#[tokio::test]
async fn generation_sends_exact_image_api_body_and_parses_outputs() {
    let (base_url, mut captured) = spawn_server(vec![(200, GENERATION_RESPONSE)]).await;
    let provider = provider_for(&base_url);

    let batch = provider
        .generate_image(MediaGenerationRequest {
            model: Some("gpt-image-2".to_string()),
            count: Some(2),
            size: Some("1024x1024".to_string()),
            quality: Some("high".to_string()),
            output_format: Some("png".to_string()),
            background: Some("transparent".to_string()),
            moderation: Some("low".to_string()),
            ..request("a tiny test image")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(raw.starts_with("POST /images/generations"), "{raw}");
    assert!(raw.contains("authorization: Bearer test-secret"), "{raw}");
    let body = body_of(&raw);
    assert_eq!(
        body,
        serde_json::json!({
            "model": "gpt-image-2",
            "prompt": "a tiny test image",
            "n": 2,
            "size": "1024x1024",
            "quality": "high",
            "output_format": "png",
            "background": "transparent",
            "moderation": "low"
        })
    );

    assert_eq!(batch.provider, "openai");
    assert_eq!(batch.model, "gpt-image-2");
    assert_eq!(batch.images.len(), 2);
    assert_eq!(batch.images[0].bytes_base64, "aW1hZ2Ux");
    assert_eq!(batch.images[0].mime_type, "image/png");
    assert_eq!(
        batch.images[0].revised_prompt.as_deref(),
        Some("a refined tiny image")
    );
    let dimensions = batch.images[0].dimensions.clone().unwrap();
    assert_eq!((dimensions.width, dimensions.height), (1024, 1024));
    let usage = batch.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(10));
    assert_eq!(usage.input_image_tokens, Some(0));
    assert_eq!(usage.output_tokens, Some(1056));
    assert_eq!(usage.total_tokens, Some(1066));
}

#[tokio::test]
async fn edit_sends_multipart_body_with_all_input_images() {
    let (base_url, mut captured) =
        spawn_server(vec![(200, r#"{"data":[{"b64_json":"ZWRpdGVk"}]}"#)]).await;
    let provider = provider_for(&base_url);

    let first = base64::engine::general_purpose::STANDARD.encode(b"first-image-bytes");
    let second = base64::engine::general_purpose::STANDARD.encode(b"second-image-bytes");
    let batch = provider
        .generate_image(MediaGenerationRequest {
            action: Some(ImageGenerationAction::Edit),
            input_images: vec![
                MediaImageInput {
                    bytes_base64: first,
                    mime_type: "image/png".to_string(),
                },
                MediaImageInput {
                    bytes_base64: second,
                    mime_type: "image/jpeg".to_string(),
                },
            ],
            size: Some("1536x1024".to_string()),
            ..request("make it a launch graphic")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(raw.starts_with("POST /images/edits"), "{raw}");
    assert!(raw.contains("multipart/form-data"), "{raw}");
    assert!(raw.contains("name=\"model\""), "{raw}");
    assert!(raw.contains("gpt-image-2"), "{raw}");
    assert!(raw.contains("name=\"prompt\""), "{raw}");
    assert!(raw.contains("make it a launch graphic"), "{raw}");
    assert!(
        raw.contains("name=\"image[]\"; filename=\"input-0.png\""),
        "{raw}"
    );
    assert!(
        raw.contains("name=\"image[]\"; filename=\"input-1.jpg\""),
        "{raw}"
    );
    assert!(raw.contains("first-image-bytes"), "{raw}");
    assert!(raw.contains("second-image-bytes"), "{raw}");
    assert!(raw.contains("name=\"size\""), "{raw}");

    assert_eq!(batch.images.len(), 1);
    assert_eq!(batch.images[0].bytes_base64, "ZWRpdGVk");
}

#[tokio::test]
async fn inline_inputs_without_action_choose_the_edit_endpoint() {
    let (base_url, mut captured) =
        spawn_server(vec![(200, r#"{"data":[{"b64_json":"ZWRpdGVk"}]}"#)]).await;
    let provider = provider_for(&base_url);

    provider
        .generate_image(MediaGenerationRequest {
            input_images: vec![MediaImageInput {
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(b"ref"),
                mime_type: "image/png".to_string(),
            }],
            ..request("auto edit")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(raw.starts_with("POST /images/edits"), "{raw}");
}

#[tokio::test]
async fn unsupported_options_fail_before_any_network_call() {
    // Unroutable base URL: validation failures must never reach the network.
    let provider = OpenAiImagesProvider::new(
        OpenAiImagesConfig::new(Some("test-secret".to_string()))
            .with_base_url("http://127.0.0.1:1"),
    );

    let unknown_model = provider
        .generate_image(MediaGenerationRequest {
            model: Some("dall-e-1".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        unknown_model.contains("unknown OpenAI image model"),
        "{unknown_model}"
    );
    assert!(unknown_model.contains("gpt-image-2"), "{unknown_model}");

    let bad_size = provider
        .generate_image(MediaGenerationRequest {
            size: Some("640x480".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        bad_size.contains("not supported by gpt-image-2"),
        "{bad_size}"
    );

    let bad_format = provider
        .generate_image(MediaGenerationRequest {
            output_format: Some("tiff".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(bad_format.contains("png, jpeg, or webp"), "{bad_format}");

    let aspect_ratio = provider
        .generate_image(MediaGenerationRequest {
            aspect_ratio: Some("16:9".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(aspect_ratio.contains("not `aspectRatio`"), "{aspect_ratio}");

    let partial = provider
        .generate_image(MediaGenerationRequest {
            partial_images: Some(2),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(partial.contains("partial image streaming"), "{partial}");

    let mut options = serde_json::Map::new();
    options.insert("seed".to_string(), serde_json::json!(7));
    let bad_option = provider
        .generate_image(MediaGenerationRequest {
            provider_options: Some(options),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        bad_option.contains("unsupported OpenAI providerOptions"),
        "{bad_option}"
    );
}

#[tokio::test]
async fn missing_api_key_fails_with_actionable_error_without_leaking_secrets() {
    let provider = OpenAiImagesProvider::new(OpenAiImagesConfig::new(None));
    let error = provider
        .generate_image(request("no key"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("OPENAI_API_KEY"), "{error}");
}

#[tokio::test]
async fn auth_failures_are_redacted_and_never_echo_the_response_body() {
    let (base_url, _captured) = spawn_server(vec![(
        401,
        r#"{"error":{"message":"Incorrect API key provided: sk-leaky-secret"}}"#,
    )])
    .await;
    let provider = provider_for(&base_url);

    let error = provider
        .generate_image(request("auth fail"))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("authentication failed (status 401)"),
        "{error}"
    );
    assert!(!error.contains("sk-leaky-secret"), "{error}");
}

#[tokio::test]
async fn provider_errors_surface_status_and_message_excerpt() {
    let (base_url, _captured) = spawn_server(vec![(
        400,
        r#"{"error":{"message":"Your organization must be verified to use gpt-image-2"}}"#,
    )])
    .await;
    let provider = provider_for(&base_url);

    let error = provider
        .generate_image(request("org gate"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("status 400"), "{error}");
    assert!(error.contains("organization must be verified"), "{error}");
}

#[tokio::test]
async fn retryable_statuses_are_retried_for_json_generation_requests() {
    let (base_url, mut captured) = spawn_server(vec![
        (429, r#"{"error":{"message":"rate limited"}}"#),
        (200, r#"{"data":[{"b64_json":"b2s="}]}"#),
    ])
    .await;
    let provider = provider_for(&base_url);

    let batch = provider.generate_image(request("retry me")).await.unwrap();

    assert_eq!(batch.images.len(), 1);
    let first = captured.recv().await.unwrap();
    let second = captured.recv().await.unwrap();
    assert!(first.starts_with("POST /images/generations"));
    assert!(second.starts_with("POST /images/generations"));
}
