//! Offline fake-HTTP coverage for the Google Gemini (Nano Banana) image
//! provider: exact generateContent bodies for all three model ids,
//! model-specific option gating before network calls, inline image output
//! parsing, SynthID watermark metadata, and redacted provider errors
//! (roadmap phase 91). No network or credentials.

use roder_api::media::{MediaGenerationRequest, MediaGeneratorProvider, MediaImageInput};
use roder_api::reliability::ReliabilityRequestPolicy;
use roder_ext_google_images::{GoogleImagesConfig, GoogleImagesProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

async fn spawn_server(
    responses: Vec<(u16, &'static str)>,
) -> (String, mpsc::Receiver<String>) {
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

fn provider_for(base_url: &str) -> GoogleImagesProvider {
    GoogleImagesProvider::new(
        GoogleImagesConfig::new(Some("test-secret".to_string()))
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

const IMAGE_RESPONSE: &str = r#"{
    "responseId": "resp-google-1",
    "candidates": [{
        "finishReason": "STOP",
        "content": {
            "parts": [
                {"text": "Here is your image."},
                {"inlineData": {"mimeType": "image/png", "data": "Z29vZ2xlLWltYWdl"}}
            ]
        }
    }],
    "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 1290, "totalTokenCount": 1302}
}"#;

#[tokio::test]
async fn nano_banana_2_sends_exact_generate_content_body() {
    let (base_url, mut captured) = spawn_server(vec![(200, IMAGE_RESPONSE)]).await;
    let provider = provider_for(&base_url);

    let batch = provider
        .generate_image(MediaGenerationRequest {
            model: Some("gemini-3.1-flash-image".to_string()),
            aspect_ratio: Some("16:9".to_string()),
            image_size: Some("2K".to_string()),
            ..request("a product hero image")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(
        raw.starts_with("POST /models/gemini-3.1-flash-image:generateContent"),
        "{raw}"
    );
    assert!(raw.contains("x-goog-api-key: test-secret"), "{raw}");
    assert!(!raw.contains("key=test-secret"), "API key must not be in the URL: {raw}");
    let body = body_of(&raw);
    assert_eq!(
        body,
        serde_json::json!({
            "contents": [{ "parts": [{ "text": "a product hero image" }] }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"],
                "imageConfig": { "aspectRatio": "16:9", "imageSize": "2K" }
            }
        })
    );

    assert_eq!(batch.provider, "google");
    assert_eq!(batch.model, "gemini-3.1-flash-image");
    assert_eq!(batch.images.len(), 1);
    assert_eq!(batch.images[0].bytes_base64, "Z29vZ2xlLWltYWdl");
    assert_eq!(batch.images[0].mime_type, "image/png");
    assert_eq!(batch.images[0].watermark.as_deref(), Some("synthid"));
    assert_eq!(batch.provider_response_id.as_deref(), Some("resp-google-1"));
    let usage = batch.usage.unwrap();
    assert_eq!(usage.input_tokens, Some(12));
    assert_eq!(usage.output_tokens, Some(1290));
    assert_eq!(usage.total_tokens, Some(1302));
}

#[tokio::test]
async fn nano_banana_pro_supports_image_size_and_reference_images() {
    let (base_url, mut captured) = spawn_server(vec![(200, IMAGE_RESPONSE)]).await;
    let provider = provider_for(&base_url);

    provider
        .generate_image(MediaGenerationRequest {
            model: Some("gemini-3-pro-image".to_string()),
            image_size: Some("4K".to_string()),
            input_images: vec![MediaImageInput {
                bytes_base64: "cmVmLWltYWdl".to_string(),
                mime_type: "image/png".to_string(),
            }],
            ..request("blend with the reference")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(
        raw.starts_with("POST /models/gemini-3-pro-image:generateContent"),
        "{raw}"
    );
    let body = body_of(&raw);
    assert_eq!(
        body,
        serde_json::json!({
            "contents": [{ "parts": [
                { "text": "blend with the reference" },
                { "inline_data": { "mime_type": "image/png", "data": "cmVmLWltYWdl" } }
            ] }],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"],
                "imageConfig": { "imageSize": "4K" }
            }
        })
    );
}

#[tokio::test]
async fn nano_banana_supports_aspect_ratio_but_not_image_size() {
    let (base_url, mut captured) = spawn_server(vec![(200, IMAGE_RESPONSE)]).await;
    let provider = provider_for(&base_url);

    provider
        .generate_image(MediaGenerationRequest {
            model: Some("gemini-2.5-flash-image".to_string()),
            aspect_ratio: Some("1:1".to_string()),
            ..request("a sticker")
        })
        .await
        .unwrap();

    let raw = captured.recv().await.unwrap();
    assert!(
        raw.starts_with("POST /models/gemini-2.5-flash-image:generateContent"),
        "{raw}"
    );
    let body = body_of(&raw);
    assert_eq!(
        body["generationConfig"]["imageConfig"],
        serde_json::json!({ "aspectRatio": "1:1" })
    );
}

#[tokio::test]
async fn model_specific_options_fail_before_any_network_call() {
    let provider = GoogleImagesProvider::new(
        GoogleImagesConfig::new(Some("test-secret".to_string()))
            .with_base_url("http://127.0.0.1:1"),
    );

    let image_size = provider
        .generate_image(MediaGenerationRequest {
            model: Some("gemini-2.5-flash-image".to_string()),
            image_size: Some("2K".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        image_size.contains("imageSize is not supported by gemini-2.5-flash-image"),
        "{image_size}"
    );

    let bad_size_tier = provider
        .generate_image(MediaGenerationRequest {
            model: Some("gemini-3-pro-image".to_string()),
            image_size: Some("8K".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(bad_size_tier.contains("supported sizes: 1K, 2K, 4K"), "{bad_size_tier}");

    let bad_ratio = provider
        .generate_image(MediaGenerationRequest {
            aspect_ratio: Some("7:3".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(bad_ratio.contains("aspect ratio \"7:3\" is not supported"), "{bad_ratio}");

    let unknown_model = provider
        .generate_image(MediaGenerationRequest {
            model: Some("imagen-3".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(unknown_model.contains("unknown Gemini image model"), "{unknown_model}");

    let pixel_size = provider
        .generate_image(MediaGenerationRequest {
            size: Some("1024x1024".to_string()),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(pixel_size.contains("not pixel `size`"), "{pixel_size}");

    let multi_output = provider
        .generate_image(MediaGenerationRequest {
            count: Some(3),
            ..request("nope")
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(multi_output.contains("one image per request"), "{multi_output}");
}

#[tokio::test]
async fn missing_api_key_mentions_both_env_fallbacks() {
    let provider = GoogleImagesProvider::new(GoogleImagesConfig::new(None));
    let error = provider
        .generate_image(request("no key"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("GEMINI_API_KEY"), "{error}");
    assert!(error.contains("GEMINI_API_TOKEN"), "{error}");
}

#[tokio::test]
async fn auth_failures_are_redacted_and_blocked_prompts_surface_reason() {
    let (base_url, _captured) = spawn_server(vec![(
        403,
        r#"{"error":{"message":"API key not valid: AIza-leaky-secret"}}"#,
    )])
    .await;
    let provider = provider_for(&base_url);
    let error = provider
        .generate_image(request("auth fail"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("authentication failed (status 403)"), "{error}");
    assert!(!error.contains("AIza-leaky-secret"), "{error}");

    let (base_url, _captured) = spawn_server(vec![(
        200,
        r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#,
    )])
    .await;
    let provider = provider_for(&base_url);
    let blocked = provider
        .generate_image(request("blocked"))
        .await
        .unwrap_err()
        .to_string();
    assert!(blocked.contains("blocked the image generation prompt: SAFETY"), "{blocked}");
}

#[tokio::test]
async fn provider_errors_surface_status_and_message_excerpt() {
    let (base_url, _captured) = spawn_server(vec![(
        429,
        r#"{"error":{"message":"Resource has been exhausted"}}"#,
    )])
    .await;
    let provider = GoogleImagesProvider::new(
        GoogleImagesConfig::new(Some("test-secret".to_string()))
            .with_base_url(&base_url)
            .with_retry_policy(ReliabilityRequestPolicy {
                provider_retry_max_attempts: 1,
                provider_retry_initial_backoff_ms: 0,
                ..ReliabilityRequestPolicy::default()
            }),
    );

    let error = provider
        .generate_image(request("quota"))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("status 429"), "{error}");
    assert!(error.contains("Resource has been exhausted"), "{error}");
}

#[tokio::test]
async fn retryable_statuses_are_retried_for_generate_content_requests() {
    let (base_url, mut captured) = spawn_server(vec![
        (503, r#"{"error":{"message":"overloaded"}}"#),
        (200, IMAGE_RESPONSE),
    ])
    .await;
    let provider = provider_for(&base_url);

    let batch = provider.generate_image(request("retry me")).await.unwrap();

    assert_eq!(batch.images.len(), 1);
    let first = captured.recv().await.unwrap();
    let second = captured.recv().await.unwrap();
    assert!(first.contains(":generateContent"));
    assert!(second.contains(":generateContent"));
}
